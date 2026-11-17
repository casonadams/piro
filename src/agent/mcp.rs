//! MCP client over stdio: the servers of `mcpServers` config files, whose tools become piro tools named
//! `mcp__<server>__<tool>`.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

use crate::agent::tool::{Tool, ToolDefinition, ToolFuture, ToolResult};

const PROTOCOL_VERSION: &str = "2025-06-18";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct Config {
    #[serde(rename = "mcpServers", default)]
    servers: BTreeMap<String, ServerConfig>,
}

#[derive(Deserialize)]
struct ServerConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

pub async fn load(configs: &[PathBuf]) -> (Vec<Arc<dyn Tool>>, Vec<String>) {
    let mut errors = Vec::new();
    let mut tools = Vec::new();
    for (name, server) in configured_servers(configs, &mut errors) {
        match tokio::time::timeout(STARTUP_TIMEOUT, start(&name, server)).await {
            Ok(Ok(found)) => tools.extend(found),
            Ok(Err(e)) => errors.push(format!("MCP server '{name}' failed: {e}")),
            Err(_) => errors.push(format!("MCP server '{name}' did not start within 30s")),
        }
    }
    (tools, errors)
}

fn configured_servers(configs: &[PathBuf], errors: &mut Vec<String>) -> BTreeMap<String, ServerConfig> {
    let mut servers = BTreeMap::new();
    for path in configs {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(config) => servers.extend(config.servers),
            Err(e) => errors.push(format!("Ignoring {}: {e}", path.display())),
        }
    }
    servers
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

struct Client {
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicU64,
    _child: Child,
}

impl Client {
    async fn send(&self, message: Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, response) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, reply);
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await?;
        response
            .await
            .map_err(|_| "the server closed its connection".to_string())?
    }
}

async fn read_responses(stdout: ChildStdout, pending: Pending) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if message.get("method").is_some() {
            continue;
        }
        let Some(reply) = message["id"]
            .as_u64()
            .and_then(|id| pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id))
        else {
            continue;
        };
        let result = match message.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap_or("unknown error").to_owned()),
            None => Ok(message["result"].clone()),
        };
        let _ = reply.send(result);
    }
}

async fn start(name: &str, server: ServerConfig) -> Result<Vec<Arc<dyn Tool>>, String> {
    let mut child = Command::new(&server.command)
        .args(&server.args)
        .envs(&server.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run '{}': {e}", server.command))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("no stdio pipes".into());
    };
    let pending = Pending::default();
    tokio::spawn(read_responses(stdout, pending.clone()));
    let client = Arc::new(Client {
        stdin: tokio::sync::Mutex::new(stdin),
        pending,
        next_id: AtomicU64::new(0),
        _child: child,
    });
    let client_info = json!({ "name": "piro", "version": env!("CARGO_PKG_VERSION") });
    let init = json!({ "protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": client_info });
    client.request("initialize", init).await?;
    client
        .send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await?;
    list_tools(name, client).await
}

async fn list_tools(server: &str, client: Arc<Client>) -> Result<Vec<Arc<dyn Tool>>, String> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = cursor.map_or_else(|| json!({}), |c| json!({ "cursor": c }));
        let page = client.request("tools/list", params).await?;
        for tool in page["tools"].as_array().into_iter().flatten() {
            tools.push(Arc::new(McpTool::new(server, tool, client.clone())));
        }
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            return Ok(tools);
        }
    }
}

struct McpTool {
    definition: ToolDefinition,
    remote_name: String,
    read_only: bool,
    client: Arc<Client>,
}

impl McpTool {
    fn new(server: &str, tool: &Value, client: Arc<Client>) -> Self {
        let remote_name = tool["name"].as_str().unwrap_or_default().to_owned();
        let name: String = format!("mcp__{server}__{remote_name}")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .take(64)
            .collect();
        let schema = tool
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" }));
        Self {
            definition: ToolDefinition {
                name,
                description: tool["description"].as_str().unwrap_or_default().to_owned(),
                schema,
            },
            remote_name,
            read_only: tool["annotations"]["readOnlyHint"].as_bool() == Some(true),
            client,
        }
    }
}

impl Tool for McpTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn run(&self, input: Value, _lines: mpsc::UnboundedSender<String>) -> ToolFuture {
        let client = self.client.clone();
        let params = json!({ "name": self.remote_name, "arguments": input });
        Box::pin(async move {
            match client.request("tools/call", params).await {
                Ok(result) => call_result(&result),
                Err(e) => ToolResult::err(e),
            }
        })
    }
}

fn call_result(result: &Value) -> ToolResult {
    let text: Vec<String> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| match item["text"].as_str() {
            Some(text) => text.to_owned(),
            None => format!("[{} content]", item["type"].as_str().unwrap_or("unknown")),
        })
        .collect();
    let text = text.join("\n");
    if result["isError"].as_bool() == Some(true) {
        ToolResult::err(text)
    } else {
        ToolResult::ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_SERVER: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) echo '{"jsonrpc":"2.0","id":'$id',"result":{"capabilities":{}}}';;
    *'"tools/list"'*) echo '{"jsonrpc":"2.0","id":'$id',"result":{"tools":[{"name":"look.up","description":"Look up","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}';;
    *'"tools/call"'*) echo '{"jsonrpc":"2.0","method":"notifications/message"}'; echo '{"jsonrpc":"2.0","id":'$id',"result":{"content":[{"type":"text","text":"found"},{"type":"image"}]}}';;
  esac
done"#;

    #[tokio::test]
    async fn lists_and_calls_tools_of_a_stdio_server() {
        let server = ServerConfig {
            command: "bash".into(),
            args: vec!["-c".into(), FAKE_SERVER.into()],
            env: HashMap::new(),
        };
        let tools = start("docs", server).await.expect("the server starts");
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.definition().name, "mcp__docs__look_up");
        assert!(tool.read_only());
        let (tx, _rx) = mpsc::unbounded_channel();
        let result = tool.run(json!({ "q": "x" }), tx).await;
        assert!(result.success());
        assert_eq!(result.output, "found\n[image content]");
    }
}
