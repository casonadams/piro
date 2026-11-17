use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::agent::tool::{Tool, ToolDefinition, ToolFuture, ToolResult, Workspace, str_arg};
use crate::sys::{ProcessHandle, RunnerMsg, ShellJob, spawn_command};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;

pub struct BashTool {
    definition: ToolDefinition,
    workspace: Workspace,
}

impl BashTool {
    pub fn new(workspace: Workspace) -> Self {
        let definition = ToolDefinition {
            name: "bash".into(),
            description: "Run a bash script in the working directory, which persists between calls. \
                stdin is closed, so avoid interactive commands."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The bash script to run" },
                    "timeout": {
                        "type": "integer",
                        "description": "Seconds before the command is killed (default 120, max 600)"
                    }
                },
                "required": ["command"]
            }),
        };
        Self { definition, workspace }
    }
}

impl Tool for BashTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    fn run(&self, input: Value, lines: mpsc::UnboundedSender<String>) -> ToolFuture {
        let workspace = self.workspace.clone();
        Box::pin(async move {
            let command = match str_arg(&input, "command") {
                Ok(command) => command.to_owned(),
                Err(e) => return ToolResult::err(e),
            };
            let secs = input
                .get("timeout")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_TIMEOUT_SECS);
            let script = Script {
                command,
                timeout: Duration::from_secs(secs.clamp(1, MAX_TIMEOUT_SECS)),
                workspace,
            };
            run_script(script, lines).await
        })
    }
}

struct Script {
    command: String,
    timeout: Duration,
    workspace: Workspace,
}

struct Running {
    process: Option<ProcessHandle>,
    cwd_file: PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.kill();
        }
        let _ = std::fs::remove_file(&self.cwd_file);
    }
}

async fn run_script(script: Script, lines: mpsc::UnboundedSender<String>) -> ToolResult {
    let cwd_file = cwd_file_path();
    let job = ShellJob {
        script: format!("trap 'pwd -P > {}' EXIT\n{}", shell_quote(&cwd_file), script.command),
        shell: Some("bash"),
        cwd: Some(script.workspace.cwd()),
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut running = Running {
        process: Some(spawn_command(job, tx)),
        cwd_file,
    };
    let mut output = Vec::new();
    let collect = async {
        let mut code = None;
        while let Some(msg) = rx.recv().await {
            match msg {
                RunnerMsg::Line(line) => {
                    let _ = lines.send(line.clone());
                    output.push(line);
                }
                RunnerMsg::Done(exit, _) => code = exit,
            }
        }
        code
    };
    let Ok(exit_code) = tokio::time::timeout(script.timeout, collect).await else {
        output.push(format!("[killed after timing out at {}s]", script.timeout.as_secs()));
        return result(output, None);
    };
    running.process.take();
    follow_cwd(&running.cwd_file, &script.workspace);
    result(output, exit_code)
}

/// Non-zero exit codes go into the output, since the model sees only the text and an error flag.
fn result(mut output: Vec<String>, exit_code: Option<i32>) -> ToolResult {
    if let Some(code) = exit_code.filter(|&code| code != 0) {
        output.push(format!("[exit code {code}]"));
    }
    ToolResult {
        output: output.join("\n"),
        exit_code,
    }
}

fn follow_cwd(cwd_file: &Path, workspace: &Workspace) {
    let Ok(dir) = std::fs::read_to_string(cwd_file) else {
        return;
    };
    let dir = PathBuf::from(dir.trim_end());
    if dir.is_dir() {
        workspace.set_cwd(dir);
    }
}

fn cwd_file_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("piro-cwd-{}-{n}", std::process::id()))
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(tool: &BashTool, input: Value) -> ToolResult {
        let (tx, _rx) = mpsc::unbounded_channel();
        tool.run(input, tx).await
    }

    #[tokio::test]
    async fn cd_persists_and_exit_codes_survive_the_cwd_trap() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let tool = BashTool::new(Workspace::new(root.clone()));
        let sub = format!("piro-bash-test-{}", std::process::id());
        let first = run(
            &tool,
            json!({ "command": format!("mkdir -p {sub} && cd {sub} && exit 3") }),
        )
        .await;
        assert_eq!(first.exit_code, Some(3));
        let pwd = run(&tool, json!({ "command": "pwd -P" })).await;
        assert_eq!(pwd.output, root.join(&sub).display().to_string());
        assert!(pwd.success());
        let _ = std::fs::remove_dir(root.join(sub));
    }

    #[tokio::test]
    async fn timeout_kills_the_command() {
        let tool = BashTool::new(Workspace::new(std::env::temp_dir()));
        let result = run(&tool, json!({ "command": "echo start; sleep 30", "timeout": 1 })).await;
        assert_eq!(result.exit_code, None);
        assert_eq!(result.output, "start\n[killed after timing out at 1s]");
    }
}
