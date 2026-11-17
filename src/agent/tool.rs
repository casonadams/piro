use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{OnceCell, mpsc};

use crate::agent::tools::{BashTool, edit_tool, read_tool, write_tool};

pub struct ToolResult {
    pub output: String,
    pub exit_code: Option<i32>,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            exit_code: Some(0),
        }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            exit_code: Some(1),
        }
    }

    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolExit {
    pub code: Option<i32>,
    pub duration: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

pub type ToolFuture = Pin<Box<dyn Future<Output = ToolResult> + Send>>;

/// Dropping the returned future cancels the tool, so tools release processes and files on drop.
pub trait Tool: Send + Sync {
    fn definition(&self) -> &ToolDefinition;
    fn read_only(&self) -> bool {
        false
    }
    fn run(&self, input: Value, lines: mpsc::UnboundedSender<String>) -> ToolFuture;
}

#[derive(Debug, Clone)]
pub struct Workspace(Arc<Mutex<PathBuf>>);

impl Workspace {
    pub fn new(cwd: PathBuf) -> Self {
        Self(Arc::new(Mutex::new(cwd)))
    }

    pub fn cwd(&self) -> PathBuf {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn set_cwd(&self, cwd: PathBuf) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = cwd;
    }

    pub fn resolve(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd().join(path)
        }
    }
}

type Tools = Vec<Arc<dyn Tool>>;

#[derive(Clone)]
pub struct ToolRegistry {
    tools: Tools,
    mcp_configs: Arc<[PathBuf]>,
    mcp: Arc<OnceCell<Tools>>,
    pub workspace: Workspace,
}

impl ToolRegistry {
    pub fn with_defaults(workspace: Workspace, mcp_configs: Vec<PathBuf>) -> Self {
        let tools: Tools = vec![
            Arc::new(BashTool::new(workspace.clone())),
            Arc::new(read_tool(workspace.clone())),
            Arc::new(write_tool(workspace.clone())),
            Arc::new(edit_tool(workspace.clone())),
        ];
        Self {
            tools,
            mcp_configs: mcp_configs.into(),
            mcp: Arc::default(),
            workspace,
        }
    }

    pub async fn load_mcp(&self, notice: impl Fn(String)) {
        let configs = self.mcp_configs.clone();
        self.mcp
            .get_or_init(|| async move {
                let (tools, errors) = crate::agent::mcp::load(&configs).await;
                errors.into_iter().for_each(notice);
                tools
            })
            .await;
    }

    fn all(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.iter().chain(self.mcp.get().into_iter().flatten())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.all().find(|t| t.definition().name == name).cloned()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.all().map(|t| t.definition().clone()).collect()
    }
}

pub fn str_arg<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing string argument '{key}' in {input}"))
}
