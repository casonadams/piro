use std::collections::HashSet;
use std::time::Duration;

use crate::agent::tool::ToolExit;
use crate::agent::{
    AgentHandle, AgentMessage, ApprovalRequest, EngineSpawnOptions, Job, ReActEngine, ToolRegistry, Workspace,
};
use crate::app::channels::Channels;
use crate::app::feed::{CommandOutputBlock, Feed, FeedItem};
use crate::app::history::{History, persist_entry};
use crate::app::route::{InputIntent, agent_prompt_for_command, parse_input_intent};
use crate::app::transcript::Transcript;
use crate::launch::SessionConfig;
use crate::provider::{Usage, provider_from_spec};
use crate::sys::{ProcessHandle, ShellJob, spawn_command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    Running,
    Thinking,
    Approval,
}

pub const BUSY_MESSAGE: &str = "Task already running, press Esc to cancel.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequest {
    Login,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Allow,
    AlwaysAllow,
    Deny,
}

/// The conversation on screen, its agent context, and the work in flight. Plain data: no UI types.
pub struct Session {
    pub feed: Feed,
    pub history: History,
    pub agent_history: Vec<AgentMessage>,
    pub tokens: Usage,
    pub context_tokens: u64,
    model_spec: String,
    tools: ToolRegistry,
    pub(crate) transcript: Transcript,
    auto_approve: bool,
    always_allowed: HashSet<String>,
    pending_approval: Option<ApprovalRequest>,
    channels: Channels,
    active_process: Option<ProcessHandle>,
    active_block: Option<u64>,
    active_agent: Option<AgentHandle>,
    pending_agent_cmd: Option<String>,
}

impl Session {
    pub fn new(config: SessionConfig, history: History, channels: Channels) -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        let data_dir = config.data_dir.as_deref();
        let mcp_configs = data_dir.map_or_else(Vec::new, |d| vec![d.join("mcp.json"), cwd.join(".mcp.json")]);
        let mut transcript = Transcript::new(data_dir.map(|d| d.join("sessions")), cwd.clone());
        let resumed = if config.resume {
            transcript.resume_latest()
        } else {
            Vec::new()
        };
        let mut session = Self {
            feed: Feed::default(),
            history,
            agent_history: Vec::new(),
            tokens: Usage::default(),
            context_tokens: 0,
            model_spec: config.model_spec,
            tools: ToolRegistry::with_defaults(Workspace::new(cwd), mcp_configs),
            transcript,
            auto_approve: config.auto_approve,
            always_allowed: HashSet::new(),
            pending_approval: None,
            channels,
            active_process: None,
            active_block: None,
            active_agent: None,
            pending_agent_cmd: None,
        };
        session.replay(resumed);
        session
    }

    pub fn status(&self) -> Status {
        if self.active_process.is_some() {
            Status::Running
        } else if self.pending_approval.is_some() {
            Status::Approval
        } else if self.active_agent.is_some() {
            Status::Thinking
        } else {
            Status::Ready
        }
    }

    pub fn is_busy(&self) -> bool {
        self.status() != Status::Ready
    }

    pub fn model_spec(&self) -> &str {
        &self.model_spec
    }

    /// Routes a submitted line: slash commands, direct and agent-bound commands, or an agent prompt.
    /// Returns what only the screen can do, such as opening the login dialog.
    pub fn submit(&mut self, raw: &str) -> Option<UiRequest> {
        match parse_input_intent(raw)? {
            InputIntent::Clear => self.clear(),
            InputIntent::Login => return Some(UiRequest::Login),
            InputIntent::Model(spec) => self.set_model(spec),
            _ if self.is_busy() => self.notice(BUSY_MESSAGE.into()),
            intent => self.start(raw.trim(), intent),
        }
        None
    }

    fn start(&mut self, entry: &str, intent: InputIntent) {
        if self.history.record(entry) {
            persist_entry(entry);
        }
        match intent {
            InputIntent::DirectCommand(cmd) => self.start_command(entry, cmd),
            InputIntent::CommandWithAgent(cmd) => {
                self.pending_agent_cmd = Some(cmd.clone());
                self.start_command(entry, cmd);
            }
            InputIntent::AgentPrompt(prompt) => self.start_agent(Job::Prompt(prompt)),
            InputIntent::Compact if self.agent_history.is_empty() => self.notice("Nothing to compact.".into()),
            InputIntent::Compact => self.start_agent(Job::Compact),
            InputIntent::Clear | InputIntent::Login | InputIntent::Model(_) => {}
        }
    }

    pub fn notice(&mut self, text: String) {
        self.feed.push(FeedItem::AgentResponse(text));
    }

    fn set_model(&mut self, spec: Option<String>) {
        let text = match spec {
            None => format!("Model: {}", self.model_spec),
            Some(spec) => match provider_from_spec(&spec) {
                Ok(_) => {
                    self.model_spec = spec;
                    format!("Model set to {}.", self.model_spec)
                }
                Err(e) => format!("Error: {e}"),
            },
        };
        self.notice(text);
    }

    pub fn clear(&mut self) {
        self.feed.clear();
        self.agent_history.clear();
        self.transcript.restart();
        self.tokens = Usage::default();
        self.context_tokens = 0;
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
        self.pending_approval = None;
        if cancelled {
            self.channels.reset();
        }
        cancelled
    }

    pub(crate) fn request_approval(&mut self, request: ApprovalRequest) {
        if self.always_allowed.contains(&request.call.name) {
            request.answer(true);
        } else {
            self.pending_approval = Some(request);
        }
    }

    pub fn answer_approval(&mut self, approval: Approval) {
        let Some(request) = self.pending_approval.take() else {
            return;
        };
        if approval == Approval::AlwaysAllow {
            self.always_allowed.insert(request.call.name.clone());
        }
        request.answer(approval != Approval::Deny);
    }

    pub fn approval_prompt(&self) -> Option<String> {
        let call = &self.pending_approval.as_ref()?.call;
        let mut text = format!("{}: {}", call.name, call.summary());
        if call.input.get("command").is_none() {
            let args = serde_json::to_string_pretty(&call.input).unwrap_or_default();
            text.extend(args.lines().take(20).map(|line| format!("\n{line}")));
        }
        Some(text)
    }

    pub fn toggle_expand(&mut self) {
        if let Some(block) = self.feed.latest_command_id().and_then(|id| self.feed.command_mut(id)) {
            block.expanded = !block.expanded;
        }
    }

    pub fn finish_agent(&mut self) {
        self.active_agent = None;
        self.pending_approval = None;
    }

    fn start_command(&mut self, entry: &str, cmd: String) {
        self.feed.push(FeedItem::UserInput(entry.to_owned()));
        let block = self
            .feed
            .push(FeedItem::CommandOutput(CommandOutputBlock::new(cmd.clone(), None)));
        let _runtime = crate::sys::runtime().enter();
        self.active_process = Some(spawn_command(ShellJob::user(cmd), self.channels.runner.clone()));
        self.active_block = Some(block);
    }

    fn start_agent(&mut self, job: Job) {
        if let Job::Prompt(prompt) = &job {
            self.feed.push(FeedItem::UserInput(prompt.clone()));
        }
        let provider = match provider_from_spec(&self.model_spec) {
            Ok(provider) => provider,
            Err(e) => return self.notice(format!("Error: {e}")),
        };
        let _runtime = crate::sys::runtime().enter();
        let engine = ReActEngine::new(provider, self.tools.clone());
        self.active_agent = Some(engine.spawn_with_options(EngineSpawnOptions {
            job,
            history: self.agent_history.clone(),
            agent_tx: self.channels.agent.clone(),
            auto_approve: self.auto_approve,
            context_tokens: self.context_tokens,
        }));
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
            self.start_agent(Job::Prompt(agent_prompt_for_command(&cmd, &output)));
        }
    }

    fn finish_active_block(&mut self, f: impl FnOnce(&mut CommandOutputBlock)) {
        if let Some(block) = self.active_block.take().and_then(|id| self.feed.command_mut(id)) {
            f(block);
        }
    }
}
