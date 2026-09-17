use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::runner::{ProcessHandle, RunnerMsg, spawn_command};

pub struct ToolResult {
    pub output: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub duration: Duration,
}

pub struct ToolRun {
    pub handle: ProcessHandle,
    pub result: JoinHandle<ToolResult>,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    #[allow(dead_code)]
    fn description(&self) -> &'static str;
    fn run(&self, args: &str, line_tx: mpsc::UnboundedSender<String>) -> ToolRun;
}

#[derive(Debug, Default, Clone)]
pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command in the terminal environment"
    }

    fn run(&self, args: &str, line_tx: mpsc::UnboundedSender<String>) -> ToolRun {
        let (proc_tx, mut proc_rx) = mpsc::unbounded_channel::<RunnerMsg>();
        let handle = spawn_command(args.to_string(), proc_tx);
        let result = tokio::spawn(async move {
            let (mut lines, mut exit_code, mut duration) = (Vec::new(), None, Duration::ZERO);
            while let Some(msg) = proc_rx.recv().await {
                match msg {
                    RunnerMsg::Line(l) => {
                        let _ = line_tx.send(l.clone());
                        lines.push(l);
                    }
                    RunnerMsg::Done(code, dur) => {
                        exit_code = code;
                        duration = dur;
                    }
                }
            }
            ToolResult {
                output: lines.join("\n"),
                success: exit_code == Some(0),
                exit_code,
                duration,
            }
        });
        ToolRun { handle, result }
    }
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn with_defaults() -> Self {
        let mut reg = Self::new();
        reg.register(Arc::new(BashTool));
        reg
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    #[allow(dead_code)]
    pub fn list(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bash_tool_and_registry() {
        let registry = ToolRegistry::with_defaults();
        let tool = registry.get("bash").expect("bash tool missing");
        assert_eq!(tool.name(), "bash");
        assert!(!tool.description().is_empty());

        let (tx, mut rx) = mpsc::unbounded_channel();
        let run = tool.run("echo 'tool test output'", tx);
        let mut lines = Vec::new();
        while let Some(l) = rx.recv().await {
            lines.push(l);
        }
        let res = run.result.await.expect("tool failed");
        assert!(res.success);
        assert_eq!(res.exit_code, Some(0));
        assert!(res.output.contains("tool test output"));
    }
}
