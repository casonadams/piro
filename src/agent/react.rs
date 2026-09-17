use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::AbortHandle;

use crate::agent::backend::{AgentBackend, AgentMessage, StepResult};
use crate::agent::tool::{Tool, ToolRegistry, ToolResult};
use crate::runner::ProcessHandle;

pub const MAX_AGENT_STEPS: usize = 10;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Thought(String),
    ToolStart { name: String, input: String },
    ToolOutput(String),
    ToolDone { exit_code: Option<i32>, duration: Duration },
    Response(String),
    Finished,
    Cancelled,
    Error(String),
}

pub struct AgentHandle {
    abort_handle: AbortHandle,
    active_tool: Arc<Mutex<Option<ProcessHandle>>>,
}

impl AgentHandle {
    pub fn cancel(&self) {
        if let Ok(mut lock) = self.active_tool.try_lock()
            && let Some(mut h) = lock.take()
        {
            h.kill();
        }
        self.abort_handle.abort();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRequest {
    pub name: String,
    pub input: String,
}

pub trait ReActHooks: Send + Sync + 'static {
    fn before_step(&mut self, _history: &[AgentMessage]) {}
    fn on_thought(&mut self, _thought: &str) {}
    fn before_tool(&mut self, _call: &ToolCallRequest) -> bool {
        true
    }
    fn after_tool(&mut self, _call: &ToolCallRequest, _result: &ToolResult) {}
    fn on_complete(&mut self, _response: &str) {}
}

#[derive(Default)]
pub struct NoopHooks;

impl ReActHooks for NoopHooks {}

pub struct AgentContext {
    pub active_tool: Arc<Mutex<Option<ProcessHandle>>>,
    pub agent_tx: mpsc::UnboundedSender<AgentEvent>,
}

async fn execute_tool_run(tool: Arc<dyn Tool>, input: &str, ctx: &AgentContext) -> Option<ToolResult> {
    let (line_tx, mut line_rx) = mpsc::unbounded_channel();
    let tool_run = tool.run(input, line_tx);
    *ctx.active_tool.lock().await = Some(tool_run.handle);

    let fwd = ctx.agent_tx.clone();
    let fwd_task = tokio::spawn(async move {
        while let Some(l) = line_rx.recv().await {
            let _ = fwd.send(AgentEvent::ToolOutput(l));
        }
    });

    let res = tool_run.result.await.ok();
    let _ = fwd_task.await;
    *ctx.active_tool.lock().await = None;

    if let Some(r) = &res {
        let _ = ctx.agent_tx.send(AgentEvent::ToolDone {
            exit_code: r.exit_code,
            duration: r.duration,
        });
    } else {
        let _ = ctx.agent_tx.send(AgentEvent::Cancelled);
    }
    res
}

struct EngineLoopState<'a, H> {
    registry: &'a ToolRegistry,
    hooks: &'a mut H,
    history: &'a mut Vec<AgentMessage>,
}

async fn dispatch_tool<H: ReActHooks>(
    call: ToolCallRequest,
    ctx: &AgentContext,
    st: &mut EngineLoopState<'_, H>,
) -> bool {
    if !st.hooks.before_tool(&call) {
        let msg = format!("Tool '{}' blocked by policy", call.name);
        let _ = ctx.agent_tx.send(AgentEvent::Error(msg.clone()));
        st.history.push(AgentMessage::Observation {
            output: msg,
            success: false,
        });
        return false;
    }

    let _ = ctx.agent_tx.send(AgentEvent::ToolStart {
        name: call.name.clone(),
        input: call.input.clone(),
    });
    st.history.push(AgentMessage::ToolCall {
        name: call.name.clone(),
        input: call.input.clone(),
    });

    let Some(t) = st.registry.get(&call.name) else {
        let err = format!("Tool '{}' not found", call.name);
        let _ = ctx.agent_tx.send(AgentEvent::Error(err.clone()));
        st.history.push(AgentMessage::Observation {
            output: err,
            success: false,
        });
        return false;
    };

    let Some(res) = execute_tool_run(t, &call.input, ctx).await else {
        return true;
    };
    st.hooks.after_tool(&call, &res);
    st.history.push(AgentMessage::Observation {
        output: res.output,
        success: res.success,
    });
    false
}

async fn handle_engine_step<H: ReActHooks>(
    step: StepResult,
    ctx: &AgentContext,
    st: &mut EngineLoopState<'_, H>,
) -> bool {
    match step {
        StepResult::Thought(t) => {
            st.hooks.on_thought(&t);
            let _ = ctx.agent_tx.send(AgentEvent::Thought(t.clone()));
            st.history.push(AgentMessage::Thought(t));
            false
        }
        StepResult::CallTool { tool: name, input } => dispatch_tool(ToolCallRequest { name, input }, ctx, st).await,
        StepResult::FinalAnswer(ans) => {
            st.hooks.on_complete(&ans);
            st.history.push(AgentMessage::Response(ans.clone()));
            let _ = ctx.agent_tx.send(AgentEvent::Response(ans));
            true
        }
    }
}

pub struct ReActEngine<B, H = NoopHooks> {
    pub backend: B,
    pub registry: ToolRegistry,
    pub hooks: H,
}

impl<B: AgentBackend + 'static> ReActEngine<B, NoopHooks> {
    pub fn new(backend: B, registry: ToolRegistry) -> Self {
        Self::with_hooks(backend, registry, NoopHooks)
    }
}

impl<B: AgentBackend + 'static, H: ReActHooks> ReActEngine<B, H> {
    pub fn with_hooks(backend: B, registry: ToolRegistry, hooks: H) -> Self {
        Self {
            backend,
            registry,
            hooks,
        }
    }

    pub fn spawn(self, prompt: String, agent_tx: mpsc::UnboundedSender<AgentEvent>) -> AgentHandle {
        let active_tool = Arc::new(Mutex::new(None));
        let ctx = AgentContext {
            active_tool: active_tool.clone(),
            agent_tx,
        };

        let task = tokio::spawn(async move {
            let mut backend = self.backend;
            let mut hooks = self.hooks;
            let _tools = self.registry.format_descriptions();
            let mut history = vec![AgentMessage::User(prompt)];

            for _ in 0..MAX_AGENT_STEPS {
                hooks.before_step(&history);
                let next = match backend.next_step(&history).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ctx.agent_tx.send(AgentEvent::Error(e));
                        break;
                    }
                };
                let mut st = EngineLoopState {
                    registry: &self.registry,
                    hooks: &mut hooks,
                    history: &mut history,
                };
                if handle_engine_step(next, &ctx, &mut st).await {
                    break;
                }
            }
            let _ = ctx.agent_tx.send(AgentEvent::Finished);
        });

        AgentHandle {
            abort_handle: task.abort_handle(),
            active_tool,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::MockAgentBackend;

    #[derive(Default)]
    struct TrackingHooks {
        thoughts: Vec<String>,
        completed: Vec<String>,
        block_tools: bool,
    }

    impl ReActHooks for TrackingHooks {
        fn on_thought(&mut self, thought: &str) {
            self.thoughts.push(thought.to_string());
        }

        fn before_tool(&mut self, _call: &ToolCallRequest) -> bool {
            !self.block_tools
        }

        fn on_complete(&mut self, response: &str) {
            self.completed.push(response.to_string());
        }
    }

    #[tokio::test]
    async fn test_react_engine_with_hooks() {
        let registry = ToolRegistry::with_defaults();
        let backend = MockAgentBackend::from_steps(vec![
            StepResult::Thought("Thinking step".into()),
            StepResult::CallTool {
                tool: "bash".into(),
                input: "echo 42".into(),
            },
            StepResult::FinalAnswer("Answer 42".into()),
        ]);

        let hooks = TrackingHooks::default();
        let engine = ReActEngine::with_hooks(backend, registry, hooks);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _handle = engine.spawn("test".into(), tx);

        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let fin = matches!(ev, AgentEvent::Finished);
            events.push(ev);
            if fin {
                break;
            }
        }
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Thought(_))));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolDone { .. })));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Response(_))));
    }

    #[tokio::test]
    async fn test_react_hooks_blocking_tool() {
        let registry = ToolRegistry::with_defaults();
        let backend = MockAgentBackend::from_steps(vec![StepResult::CallTool {
            tool: "bash".into(),
            input: "echo blocked".into(),
        }]);

        let hooks = TrackingHooks {
            thoughts: Vec::new(),
            completed: Vec::new(),
            block_tools: true,
        };
        let engine = ReActEngine::with_hooks(backend, registry, hooks);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _handle = engine.spawn("test".into(), tx);

        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let fin = matches!(ev, AgentEvent::Finished);
            events.push(ev);
            if fin {
                break;
            }
        }
        assert!(events.iter().any(|e| match e {
            AgentEvent::Error(err) => err.contains("blocked by policy"),
            _ => false,
        }));
    }
}
