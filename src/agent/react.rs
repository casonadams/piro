use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::agent::message::{AgentMessage, AssistantTurn, ToolCall, close_dangling_tool_calls};
use crate::agent::tool::{ToolDefinition, ToolExit, ToolRegistry, ToolResult};
use crate::provider::{ChatRequestParams, LlmProvider};
use crate::sys::ProcessHandle;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    ThoughtDelta(String),
    ResponseDelta(String),
    ToolStart(ToolCall),
    ToolOutput {
        call_id: String,
        line: String,
    },
    ToolDone {
        call_id: String,
        exit: ToolExit,
    },
    /// A message the engine appended to the conversation; the session mirrors these.
    Committed(AgentMessage),
    Notice(String),
    Finished,
    Cancelled,
    Error(String),
}

type ActiveTool = Arc<Mutex<Option<ProcessHandle>>>;

pub struct AgentHandle {
    abort_handle: AbortHandle,
    active_tool: ActiveTool,
}

impl AgentHandle {
    pub fn cancel(&self) {
        if let Some(mut h) = self.active_tool.lock().unwrap_or_else(PoisonError::into_inner).take() {
            h.kill();
        }
        self.abort_handle.abort();
    }

    #[cfg(test)]
    pub fn mock(abort_handle: AbortHandle) -> Self {
        Self {
            abort_handle,
            active_tool: Arc::default(),
        }
    }
}

pub trait ReActHooks: Send + Sync + 'static {
    fn before_step(&mut self, _history: &[AgentMessage]) {}
    fn on_thought(&mut self, _thought: &str) {}
    fn before_tool(&mut self, _call: &ToolCall) -> bool {
        true
    }
    fn after_tool(&mut self, _call: &ToolCall, _result: &ToolResult) {}
    fn on_complete(&mut self, _response: &str) {}
}

#[derive(Default)]
pub struct NoopHooks;

impl ReActHooks for NoopHooks {}

const MAX_OBSERVATION_LINES: usize = 200;
const HEAD_LINES: usize = 120;
const TAIL_LINES: usize = 79;

pub fn truncate_observation_output(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= MAX_OBSERVATION_LINES {
        return output.to_string();
    }
    let omitted = lines.len() - HEAD_LINES - TAIL_LINES;
    let mut truncated = Vec::with_capacity(MAX_OBSERVATION_LINES + 1);
    truncated.extend_from_slice(&lines[..HEAD_LINES]);
    let notice = format!("... [output truncated: {} lines omitted] ...", omitted);
    truncated.push(&notice);
    truncated.extend_from_slice(&lines[lines.len() - TAIL_LINES..]);
    truncated.join("\n")
}

pub const DEFAULT_MAX_AGENT_STEPS: usize = 1000;

pub fn resolved_max_steps() -> Option<usize> {
    match std::env::var("PIRO_MAX_STEPS") {
        Ok(val) => match val.trim().to_lowercase().as_str() {
            "0" | "none" | "unlimited" | "infinite" => None,
            s => s.parse::<usize>().ok().or(Some(DEFAULT_MAX_AGENT_STEPS)),
        },
        Err(_) => Some(DEFAULT_MAX_AGENT_STEPS),
    }
}

#[derive(Default)]
struct RepetitionGuard {
    last_call: Option<(String, String)>,
    count: usize,
}

impl RepetitionGuard {
    fn is_repetitive(&mut self, tool: &str, input: &str) -> bool {
        if self.last_call.as_ref().map(|(t, i)| (t.as_str(), i.as_str())) == Some((tool, input)) {
            self.count += 1;
            self.count >= 3
        } else {
            self.last_call = Some((tool.to_string(), input.to_string()));
            self.count = 1;
            false
        }
    }

    fn first_repetitive<'a>(&mut self, calls: &'a [ToolCall]) -> Option<&'a ToolCall> {
        calls.iter().find(|c| self.is_repetitive(&c.name, &c.input))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopAction {
    Continue,
    Finish,
}

struct EngineState<P, H> {
    provider: P,
    registry: ToolRegistry,
    tools: Vec<ToolDefinition>,
    hooks: H,
    history: Vec<AgentMessage>,
    active_tool: ActiveTool,
    agent_tx: mpsc::UnboundedSender<AgentEvent>,
}

impl<P: LlmProvider, H: ReActHooks> EngineState<P, H> {
    fn emit(&self, event: AgentEvent) {
        let _ = self.agent_tx.send(event);
    }

    fn commit(&mut self, msg: AgentMessage) {
        self.emit(AgentEvent::Committed(msg.clone()));
        self.history.push(msg);
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

    async fn request_turn(&mut self) -> Result<AssistantTurn, String> {
        self.hooks.before_step(&self.history);
        self.provider
            .chat_stream(ChatRequestParams {
                history: &self.history,
                tools: &self.tools,
                delta_tx: Some(&self.agent_tx),
            })
            .await
    }

    async fn execute(&self, call: &ToolCall) -> Option<ToolResult> {
        let tool = self.registry.get(&call.name)?;
        let (line_tx, mut line_rx) = mpsc::unbounded_channel();
        let run = tool.run(&call.input, line_tx);
        *self.active_tool.lock().unwrap_or_else(PoisonError::into_inner) = Some(run.handle);
        while let Some(line) = line_rx.recv().await {
            self.emit(AgentEvent::ToolOutput {
                call_id: call.id.clone(),
                line,
            });
        }
        let result = run.result.await.ok();
        self.active_tool.lock().unwrap_or_else(PoisonError::into_inner).take();
        result
    }

    async fn dispatch_tool(&mut self, call: ToolCall) -> LoopAction {
        if !self.hooks.before_tool(&call) {
            let msg = format!("Tool '{}' blocked by policy", call.name);
            return self.reject(call, msg);
        }
        if self.registry.get(&call.name).is_none() {
            let msg = format!("Tool '{}' not found", call.name);
            return self.reject(call, msg);
        }
        self.emit(AgentEvent::ToolStart(call.clone()));
        let Some(res) = self.execute(&call).await else {
            self.emit(AgentEvent::Cancelled);
            return LoopAction::Finish;
        };
        self.emit(AgentEvent::ToolDone {
            call_id: call.id.clone(),
            exit: ToolExit {
                code: res.exit_code,
                duration: res.duration,
            },
        });
        self.hooks.after_tool(&call, &res);
        self.commit(AgentMessage::ToolResult {
            call_id: call.id,
            output: truncate_observation_output(&res.output),
            success: res.success,
        });
        LoopAction::Continue
    }

    async fn step(&mut self, guard: &mut RepetitionGuard) -> LoopAction {
        let turn = match self.request_turn().await {
            Ok(turn) => turn,
            Err(e) => {
                self.emit(AgentEvent::Error(e));
                return LoopAction::Finish;
            }
        };
        if let Some(call) = guard.first_repetitive(&turn.tool_calls) {
            let msg = format!("Terminated repetitive loop: tool '{}' called repeatedly.", call.name);
            self.emit(AgentEvent::Notice(msg));
            return LoopAction::Finish;
        }
        if let Some(thinking) = &turn.thinking {
            self.hooks.on_thought(&thinking.text);
        }
        let calls = turn.tool_calls.clone();
        if calls.is_empty() {
            self.hooks.on_complete(turn.text.as_deref().unwrap_or_default());
        }
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

    async fn run(mut self) {
        let max_steps = resolved_max_steps();
        let mut guard = RepetitionGuard::default();
        let mut steps_taken = 0;
        loop {
            if max_steps.is_some_and(|limit| steps_taken >= limit) {
                let msg = format!("Reached maximum step limit ({} steps).", steps_taken);
                self.emit(AgentEvent::Notice(msg));
                break;
            }
            steps_taken += 1;
            if self.step(&mut guard).await == LoopAction::Finish {
                break;
            }
        }
        self.emit(AgentEvent::Finished);
    }
}

pub struct EngineSpawnOptions {
    pub prompt: String,
    pub history: Vec<AgentMessage>,
    pub agent_tx: mpsc::UnboundedSender<AgentEvent>,
}

pub struct ReActEngine<P, H = NoopHooks> {
    pub provider: P,
    pub registry: ToolRegistry,
    pub hooks: H,
}

impl<P: LlmProvider> ReActEngine<P, NoopHooks> {
    pub fn new(provider: P, registry: ToolRegistry) -> Self {
        Self::with_hooks(provider, registry, NoopHooks)
    }
}

impl<P: LlmProvider, H: ReActHooks> ReActEngine<P, H> {
    pub fn with_hooks(provider: P, registry: ToolRegistry, hooks: H) -> Self {
        Self {
            provider,
            registry,
            hooks,
        }
    }

    pub fn spawn_with_options(self, opts: EngineSpawnOptions) -> AgentHandle {
        let active_tool = ActiveTool::default();
        let mut history = opts.history;
        close_dangling_tool_calls(&mut history);
        let mut state = EngineState {
            provider: self.provider,
            tools: self.registry.definitions(),
            registry: self.registry,
            hooks: self.hooks,
            history,
            active_tool: active_tool.clone(),
            agent_tx: opts.agent_tx,
        };
        state.commit(AgentMessage::User(opts.prompt));
        let task = crate::sys::runtime().spawn(state.run());
        AgentHandle {
            abort_handle: task.abort_handle(),
            active_tool,
        }
    }

    #[cfg(test)]
    pub fn spawn(self, prompt: String, agent_tx: mpsc::UnboundedSender<AgentEvent>) -> AgentHandle {
        self.spawn_with_options(EngineSpawnOptions {
            prompt,
            history: Vec::new(),
            agent_tx,
        })
    }
}

#[cfg(test)]
mod tests;
