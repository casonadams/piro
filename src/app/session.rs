use std::time::Duration;

use crate::agent::tool::ToolExit;
use crate::agent::{AgentHandle, AgentMessage, EngineSpawnOptions, ReActEngine, ToolRegistry};
use crate::app::channels::Channels;
use crate::app::feed::{CommandOutputBlock, Feed, FeedItem};
use crate::app::history::{History, persist_entry};
use crate::app::route::{InputIntent, agent_prompt_for_command, parse_input_intent};
use crate::sys::{ProcessHandle, spawn_command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    Running,
    Thinking,
}

pub const BUSY_MESSAGE: &str = "Task already running, press Esc to cancel.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequest {
    Login,
}

/// The conversation on screen, its agent context, and the work in flight. Plain data: no UI types.
pub struct Session {
    pub feed: Feed,
    pub history: History,
    pub agent_history: Vec<AgentMessage>,
    model_spec: String,
    channels: Channels,
    active_process: Option<ProcessHandle>,
    active_block: Option<u64>,
    active_agent: Option<AgentHandle>,
    pending_agent_cmd: Option<String>,
}

impl Session {
    pub fn new(model_spec: String, history: History, channels: Channels) -> Self {
        Self {
            feed: Feed::default(),
            history,
            agent_history: Vec::new(),
            model_spec,
            channels,
            active_process: None,
            active_block: None,
            active_agent: None,
            pending_agent_cmd: None,
        }
    }

    pub fn status(&self) -> Status {
        if self.active_process.is_some() {
            Status::Running
        } else if self.active_agent.is_some() {
            Status::Thinking
        } else {
            Status::Ready
        }
    }

    pub fn is_busy(&self) -> bool {
        self.status() != Status::Ready
    }

    /// Routes a submitted line: slash commands, direct and agent-bound commands, or an agent prompt.
    /// Returns what only the screen can do, such as opening the login dialog.
    pub fn submit(&mut self, raw: &str) -> Option<UiRequest> {
        let intent = parse_input_intent(raw)?;
        match intent {
            InputIntent::Clear => {
                self.clear();
                return None;
            }
            InputIntent::Login => return Some(UiRequest::Login),
            _ => {}
        }
        if self.is_busy() {
            self.feed.push(FeedItem::AgentResponse(BUSY_MESSAGE.into()));
            return None;
        }
        let entry = raw.trim();
        if self.history.record(entry) {
            persist_entry(entry);
        }
        match intent {
            InputIntent::DirectCommand(cmd) => self.start_command(entry, cmd),
            InputIntent::CommandWithAgent(cmd) => {
                self.pending_agent_cmd = Some(cmd.clone());
                self.start_command(entry, cmd);
            }
            InputIntent::AgentPrompt(prompt) => self.start_agent(prompt),
            InputIntent::Clear | InputIntent::Login => {}
        }
        None
    }

    pub fn notice(&mut self, text: String) {
        self.feed.push(FeedItem::AgentResponse(text));
    }

    pub fn clear(&mut self) {
        self.feed.clear();
        self.agent_history.clear();
    }

    /// Ends the running command and agent turn. Returns whether anything was running.
    pub fn cancel(&mut self) -> bool {
        let mut cancelled = false;
        if let Some(agent) = self.active_agent.take() {
            agent.cancel();
            cancelled = true;
        }
        if let Some(mut process) = self.active_process.take() {
            process.kill();
            self.finish_active_block(|block| block.cancel());
            cancelled = true;
        }
        self.pending_agent_cmd = None;
        if cancelled {
            self.channels.reset();
        }
        cancelled
    }

    pub fn toggle_expand(&mut self) {
        if let Some(block) = self.feed.latest_command_id().and_then(|id| self.feed.command_mut(id)) {
            block.expanded = !block.expanded;
        }
    }

    pub fn finish_agent(&mut self) {
        self.active_agent = None;
    }

    fn start_command(&mut self, entry: &str, cmd: String) {
        self.feed.push(FeedItem::UserInput(entry.to_owned()));
        let block = self
            .feed
            .push(FeedItem::CommandOutput(CommandOutputBlock::new(cmd.clone(), None)));
        let _runtime = crate::sys::runtime().enter();
        self.active_process = Some(spawn_command(cmd, self.channels.runner.clone()));
        self.active_block = Some(block);
    }

    fn start_agent(&mut self, prompt: String) {
        self.feed.push(FeedItem::UserInput(prompt.clone()));
        let provider = match crate::provider::provider_from_spec(&self.model_spec) {
            Ok(provider) => provider,
            Err(e) => {
                self.feed.push(FeedItem::AgentResponse(format!("Error: {e}")));
                return;
            }
        };
        let _runtime = crate::sys::runtime().enter();
        self.active_agent = Some(
            ReActEngine::new(provider, ToolRegistry::with_defaults()).spawn_with_options(EngineSpawnOptions {
                prompt,
                history: self.agent_history.clone(),
                agent_tx: self.channels.agent.clone(),
            }),
        );
    }

    pub(crate) fn push_command_line(&mut self, line: String) {
        if let Some(block) = self.active_block.and_then(|id| self.feed.command_mut(id)) {
            block.push_line(line);
        }
    }

    /// Completes the running command's block; an agent-bound `!cmd` then hands its output to the agent.
    pub(crate) fn command_finished(&mut self, code: Option<i32>, duration: Duration) {
        self.active_process = None;
        let output = self
            .active_block
            .take()
            .and_then(|id| self.feed.command_mut(id))
            .map(|block| {
                block.complete(ToolExit { code, duration });
                block.copy_text()
            })
            .unwrap_or_default();
        if let Some(cmd) = self.pending_agent_cmd.take() {
            self.start_agent(agent_prompt_for_command(&cmd, &output));
        }
    }

    fn finish_active_block(&mut self, f: impl FnOnce(&mut CommandOutputBlock)) {
        if let Some(block) = self.active_block.take().and_then(|id| self.feed.command_mut(id)) {
            f(block);
        }
    }
}
