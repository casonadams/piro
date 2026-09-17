use std::time::{Duration, Instant};

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
            status: CommandStatus::Running,
            lines: Vec::new(),
            started_at: Instant::now(),
            duration: None,
            expanded: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum FeedItem {
    UserInput(String),
    CommandOutput(CommandOutputBlock),
    AgentThought(String),
    AgentResponse(String),
}
