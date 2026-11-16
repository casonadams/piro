use std::time::{Duration, Instant};

use crate::agent::tool::ToolExit;

const MAX_BLOCK_LINES: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandStatus {
    Running,
    Success(i32),
    Failed(i32),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct CommandOutputBlock {
    pub command: String,
    pub tool: Option<String>,
    pub call_id: Option<String>,
    pub status: CommandStatus,
    pub lines: Vec<String>,
    pub started_at: Instant,
    pub duration: Option<Duration>,
    pub expanded: bool,
}

impl CommandOutputBlock {
    pub fn new(command: String, tool: Option<String>) -> Self {
        Self {
            command,
            tool,
            call_id: None,
            status: CommandStatus::Running,
            lines: Vec::new(),
            started_at: Instant::now(),
            duration: None,
            expanded: false,
        }
    }

    pub fn push_line(&mut self, line: String) {
        if self.lines.len() >= MAX_BLOCK_LINES {
            self.lines.remove(0);
        }
        self.lines.push(line);
    }

    pub fn complete(&mut self, exit: ToolExit) {
        self.duration = Some(exit.duration);
        self.status = match exit.code {
            Some(0) => CommandStatus::Success(0),
            Some(c) => CommandStatus::Failed(c),
            None => CommandStatus::Failed(-1),
        };
    }
}

#[derive(Debug, Clone)]
pub enum FeedItem {
    UserInput(String),
    CommandOutput(CommandOutputBlock),
    AgentThought(String),
    AgentResponse(String),
}
