#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::runner::{ProcessHandle, RunnerMsg, spawn_command};

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    #[allow(dead_code)]
    pub duration: Duration,
}

pub struct ToolRun {
    #[allow(dead_code)]
    pub handle: ProcessHandle,
    pub result: JoinHandle<ToolResult>,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    #[allow(dead_code)]
    fn description(&self) -> &'static str;
    fn run(&self, args: &str, ui_tx: mpsc::UnboundedSender<RunnerMsg>) -> ToolRun;
}

#[derive(Debug, Default, Clone)]
pub struct BashTool;

impl BashTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command in the terminal environment"
    }

    fn run(&self, args: &str, ui_tx: mpsc::UnboundedSender<RunnerMsg>) -> ToolRun {
        let (proc_tx, mut proc_rx) = mpsc::unbounded_channel::<RunnerMsg>();
        let handle = spawn_command(args.to_string(), proc_tx);

        let result = tokio::spawn(async move {
            let mut lines = Vec::new();
            let mut exit_code = None;
            let mut duration = Duration::ZERO;

            while let Some(msg) = proc_rx.recv().await {
                match &msg {
                    RunnerMsg::Line(l) => {
                        lines.push(l.clone());
                    }
                    RunnerMsg::Done(code, dur) => {
                        exit_code = *code;
                        duration = *dur;
                    }
                }
                let _ = ui_tx.send(msg);
            }

            let success = exit_code == Some(0);
            ToolResult {
                output: lines.join("\n"),
                success,
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
        reg.register(Arc::new(BashTool::new()));
        reg
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    pub fn list(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bash_tool_success() {
        let tool = BashTool::new();
        assert!(!tool.description().is_empty());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let run = tool.run("echo 'hello agent tool'", tx);

        // Receive forwarded runner messages
        let mut forwarded = Vec::new();
        while let Some(msg) = rx.recv().await {
            forwarded.push(msg);
        }

        let res = run.result.await.expect("join failed");
        assert!(res.success);
        assert_eq!(res.exit_code, Some(0));
        assert_eq!(res.output.trim(), "hello agent tool");
        assert!(!forwarded.is_empty());
    }

    #[tokio::test]
    async fn test_bash_tool_failure() {
        let tool = BashTool::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let run = tool.run("false", tx);

        while rx.recv().await.is_some() {}

        let res = run.result.await.expect("join failed");
        assert!(!res.success);
        assert_ne!(res.exit_code, Some(0));
    }

    #[test]
    fn test_tool_registry() {
        let reg = ToolRegistry::with_defaults();
        assert!(reg.get("bash").is_some());
        assert!(reg.get("nonexistent").is_none());
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].name(), "bash");
    }
}
