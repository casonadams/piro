use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::agent::event::{AgentEvent, AgentHandle, ApprovalRequest};
use crate::agent::limits::{RepetitionGuard, resolved_max_steps};
use crate::agent::message::{AgentMessage, AssistantTurn, ToolCall, close_dangling_tool_calls};
use crate::agent::observation::observation;
use crate::agent::subagent::{TASK_TOOL, task_definition};
use crate::agent::tool::{Tool, ToolDefinition, ToolExit, ToolRegistry, ToolResult};
use crate::provider::{ChatRequestParams, LlmProvider, Reply};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopAction {
    Continue,
    /// The model only reasoned: no text and no tool call. Ask again without committing the turn.
    Retry,
    Finish,
}

const MAX_SILENT_TURNS: usize = 3;
const SILENT_NOTICE: &str = "The model stopped without an answer or a tool call.";
const MAX_RETRIES: u32 = 4;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
const TRUNCATED_NOTICE: &str = "The reply hit the output token limit; asking the model to continue.";
const TRUNCATED_PROMPT: &str = "Your last reply was cut off at the output token limit and its tool calls were \
dropped. Continue in smaller steps, for example by writing large files in parts.";
const DENIED: &str = "The user denied this tool call.";

pub(super) struct EngineState<P> {
    pub(super) provider: Arc<P>,
    pub(super) registry: ToolRegistry,
    pub(super) tools: Vec<ToolDefinition>,
    pub(super) system: String,
    pub(super) history: Vec<AgentMessage>,
    pub(super) agent_tx: mpsc::UnboundedSender<AgentEvent>,
    pub(super) auto_approve: bool,
    pub(super) context_tokens: u64,
    pub(super) depth: u8,
}

impl<P: LlmProvider> EngineState<P> {
    pub(super) fn emit(&self, event: AgentEvent) {
        let _ = self.agent_tx.send(event);
    }

    pub(super) fn commit(&mut self, msg: AgentMessage) {
        self.emit(AgentEvent::Committed(msg.clone()));
        self.history.push(msg);
    }

    pub(super) fn params(&self) -> ChatRequestParams<'_> {
        ChatRequestParams {
            system: &self.system,
            history: &self.history,
            tools: &self.tools,
            delta_tx: Some(&self.agent_tx),
        }
    }

    fn reject(&mut self, call: ToolCall, msg: String) -> LoopAction {
        self.emit(AgentEvent::Error(msg.clone()));
        self.commit(AgentMessage::ToolResult {
            call_id: call.id,
            output: msg,
            success: false,
        });
        LoopAction::Continue
    }

    async fn request_turn(&self) -> Result<Reply, String> {
        let mut attempt = 0;
        loop {
            match self.provider.chat_stream(self.params()).await {
                Err(e) if e.retryable && attempt < MAX_RETRIES => {
                    attempt += 1;
                    let delay = e
                        .retry_after
                        .unwrap_or(Duration::from_secs(1 << attempt))
                        .min(MAX_RETRY_DELAY);
                    let msg = format!(
                        "{} Retrying in {}s ({attempt}/{MAX_RETRIES}).",
                        e.message,
                        delay.as_secs()
                    );
                    self.emit(AgentEvent::Notice(msg));
                    tokio::time::sleep(delay).await;
                }
                other => return other.map_err(|e| e.message),
            }
        }
    }

    async fn approve(&self, call: &ToolCall) -> bool {
        if self.auto_approve {
            return true;
        }
        let (request, answer) = ApprovalRequest::new(call.clone());
        self.emit(AgentEvent::Approval(request));
        answer.await.unwrap_or(false)
    }

    async fn execute(&self, tool: Arc<dyn Tool>, call: &ToolCall) -> ToolResult {
        let (line_tx, mut line_rx) = mpsc::unbounded_channel();
        let mut run = tool.run(call.input.clone(), line_tx);
        let forward = |line| {
            self.emit(AgentEvent::ToolOutput {
                call_id: call.id.clone(),
                line,
            })
        };
        loop {
            tokio::select! {
                result = &mut run => {
                    while let Ok(line) = line_rx.try_recv() {
                        forward(line);
                    }
                    return result;
                }
                Some(line) = line_rx.recv() => forward(line),
            }
        }
    }

    async fn dispatch_tool(&mut self, call: ToolCall) -> LoopAction {
        let tool = self.registry.get(&call.name);
        let is_task = self.depth == 0 && call.name == TASK_TOOL;
        if tool.is_none() && !is_task {
            let msg = format!("Tool '{}' not found", call.name);
            return self.reject(call, msg);
        }
        if tool.as_ref().is_some_and(|t| !t.read_only()) && !self.approve(&call).await {
            return self.reject(call, DENIED.into());
        }
        self.emit(AgentEvent::ToolStart(call.clone()));
        let started = Instant::now();
        let result = match tool {
            Some(tool) => self.execute(tool, &call).await,
            None => self.run_task(&call).await,
        };
        self.emit(AgentEvent::ToolDone {
            call_id: call.id.clone(),
            exit: ToolExit {
                code: result.exit_code,
                duration: started.elapsed(),
            },
        });
        self.commit(AgentMessage::ToolResult {
            call_id: call.id,
            output: observation(&result),
            success: result.success(),
        });
        LoopAction::Continue
    }

    fn continue_truncated(&mut self, mut turn: AssistantTurn) -> LoopAction {
        turn.tool_calls.clear();
        if turn.text.is_some() {
            self.commit(AgentMessage::Assistant(turn));
        }
        self.emit(AgentEvent::Notice(TRUNCATED_NOTICE.into()));
        self.commit(AgentMessage::User(TRUNCATED_PROMPT.into()));
        LoopAction::Continue
    }

    async fn step(&mut self, guard: &mut RepetitionGuard) -> LoopAction {
        let reply = match self.request_turn().await {
            Ok(reply) => reply,
            Err(e) => {
                self.emit(AgentEvent::Error(e));
                return LoopAction::Finish;
            }
        };
        self.context_tokens = reply.usage.context();
        self.emit(AgentEvent::Usage(reply.usage));
        if reply.truncated {
            return self.continue_truncated(reply.turn);
        }
        let turn = reply.turn;
        if let Some(call) = guard.first_repetitive(&turn.tool_calls) {
            let msg = format!("Terminated repetitive loop: tool '{}' called repeatedly.", call.name);
            self.emit(AgentEvent::Notice(msg));
            return LoopAction::Finish;
        }
        if turn.tool_calls.is_empty() && turn.text.is_none() {
            return LoopAction::Retry;
        }
        let calls = turn.tool_calls.clone();
        self.commit(AgentMessage::Assistant(turn));
        if calls.is_empty() {
            return LoopAction::Finish;
        }
        for call in calls {
            if self.dispatch_tool(call).await == LoopAction::Finish {
                return LoopAction::Finish;
            }
        }
        LoopAction::Continue
    }

    pub(super) async fn react(&mut self) {
        let max_steps = resolved_max_steps();
        let mut guard = RepetitionGuard::default();
        let (mut steps_taken, mut silent_turns) = (0, 0);
        loop {
            if max_steps.is_some_and(|limit| steps_taken >= limit) {
                let msg = format!("Reached maximum step limit ({steps_taken} steps).");
                self.emit(AgentEvent::Notice(msg));
                break;
            }
            steps_taken += 1;
            self.compact_if_full().await;
            match self.step(&mut guard).await {
                LoopAction::Finish => break,
                LoopAction::Continue => silent_turns = 0,
                LoopAction::Retry => silent_turns += 1,
            }
            if silent_turns >= MAX_SILENT_TURNS {
                self.emit(AgentEvent::Notice(SILENT_NOTICE.into()));
                break;
            }
        }
    }

    async fn run(mut self, job: Job) {
        let tx = self.agent_tx.clone();
        self.registry
            .load_mcp(move |e| {
                let _ = tx.send(AgentEvent::Notice(e));
            })
            .await;
        self.tools = self.registry.definitions();
        self.tools.push(task_definition());
        match job {
            Job::Prompt(prompt) => {
                self.compact_if_full().await;
                self.commit(AgentMessage::User(prompt));
                self.react().await;
            }
            Job::Compact => self.compact().await,
        }
        self.emit(AgentEvent::Finished);
    }
}

pub enum Job {
    Prompt(String),
    Compact,
}

pub struct EngineSpawnOptions {
    pub job: Job,
    pub history: Vec<AgentMessage>,
    pub agent_tx: mpsc::UnboundedSender<AgentEvent>,
    pub auto_approve: bool,
    /// Lets a long resumed conversation compact before its first request.
    pub context_tokens: u64,
}

pub struct ReActEngine<P> {
    pub provider: P,
    pub registry: ToolRegistry,
}

impl<P: LlmProvider> ReActEngine<P> {
    pub fn new(provider: P, registry: ToolRegistry) -> Self {
        Self { provider, registry }
    }

    pub fn spawn_with_options(self, opts: EngineSpawnOptions) -> AgentHandle {
        let mut history = opts.history;
        close_dangling_tool_calls(&mut history);
        let state = EngineState {
            provider: Arc::new(self.provider),
            system: crate::agent::prompt::system_prompt(&self.registry.workspace.cwd()),
            registry: self.registry,
            tools: Vec::new(),
            history,
            agent_tx: opts.agent_tx,
            auto_approve: opts.auto_approve,
            context_tokens: opts.context_tokens,
            depth: 0,
        };
        let task = crate::sys::runtime().spawn(state.run(opts.job));
        AgentHandle {
            abort_handle: task.abort_handle(),
        }
    }
}

#[cfg(test)]
mod tests;
