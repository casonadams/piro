pub mod backend;
pub mod tool;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::AbortHandle;

pub use backend::{AgentBackend, AgentMessage, MockAgentBackend, StepResult};
pub use tool::{Tool, ToolRegistry, ToolResult};

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

pub struct AgentContext {
    pub active_tool: Arc<Mutex<Option<ProcessHandle>>>,
    pub agent_tx: mpsc::UnboundedSender<AgentEvent>,
}

pub struct AgentConfig<B> {
    pub prompt: String,
    pub backend: B,
    pub registry: ToolRegistry,
}

async fn execute_agent_tool(tool: Arc<dyn Tool>, input: &str, ctx: &AgentContext) -> Option<ToolResult> {
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

struct AgentLoopState<'a> {
    registry: &'a ToolRegistry,
    history: &'a mut Vec<AgentMessage>,
}

struct ToolCallRequest {
    pub name: String,
    pub input: String,
}

async fn dispatch_tool(call: ToolCallRequest, ctx: &AgentContext, st: &mut AgentLoopState<'_>) -> bool {
    let _ = ctx.agent_tx.send(AgentEvent::ToolStart {
        name: call.name.clone(),
        input: call.input.clone(),
    });
    st.history.push(AgentMessage::ToolCall {
        name: call.name.clone(),
        input: call.input.clone(),
    });
    if let Some(t) = st.registry.get(&call.name) {
        let Some(res) = execute_agent_tool(t, &call.input, ctx).await else {
            return true;
        };
        st.history.push(AgentMessage::Observation {
            output: res.output,
            success: res.success,
        });
    } else {
        let err = format!("Tool '{}' not found", call.name);
        let _ = ctx.agent_tx.send(AgentEvent::Error(err.clone()));
        st.history.push(AgentMessage::Observation {
            output: err,
            success: false,
        });
    }
    false
}

async fn handle_agent_step(step: StepResult, ctx: &AgentContext, st: &mut AgentLoopState<'_>) -> bool {
    match step {
        StepResult::Thought(t) => {
            let _ = ctx.agent_tx.send(AgentEvent::Thought(t.clone()));
            st.history.push(AgentMessage::Thought(t));
            false
        }
        StepResult::CallTool { tool: name, input } => dispatch_tool(ToolCallRequest { name, input }, ctx, st).await,
        StepResult::FinalAnswer(ans) => {
            st.history.push(AgentMessage::Response(ans.clone()));
            let _ = ctx.agent_tx.send(AgentEvent::Response(ans));
            true
        }
    }
}

pub fn spawn_agent<B: AgentBackend + 'static>(
    config: AgentConfig<B>,
    agent_tx: mpsc::UnboundedSender<AgentEvent>,
) -> AgentHandle {
    let active_tool = Arc::new(Mutex::new(None));
    let ctx = AgentContext {
        active_tool: active_tool.clone(),
        agent_tx,
    };

    let task = tokio::spawn(async move {
        let mut backend = config.backend;
        let _tools = config.registry.format_descriptions();
        let mut history = vec![AgentMessage::User(config.prompt)];
        for _ in 0..MAX_AGENT_STEPS {
            let next = match backend.next_step(&history).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = ctx.agent_tx.send(AgentEvent::Error(e));
                    break;
                }
            };
            let mut st = AgentLoopState {
                registry: &config.registry,
                history: &mut history,
            };
            if handle_agent_step(next, &ctx, &mut st).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_loop_execution() {
        let registry = ToolRegistry::with_defaults();
        let backend = MockAgentBackend::from_steps(vec![
            StepResult::Thought("Thinking".into()),
            StepResult::CallTool {
                tool: "bash".into(),
                input: "echo 'hello test'".into(),
            },
            StepResult::FinalAnswer("Finished".into()),
        ]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let config = AgentConfig {
            prompt: "count files".into(),
            backend,
            registry,
        };
        let _handle = spawn_agent(config, tx);

        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let is_fin = matches!(ev, AgentEvent::Finished);
            events.push(ev);
            if is_fin {
                break;
            }
        }
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Thought(_))));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolStart { .. })));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolOutput(_))));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolDone { .. })));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Response(_))));
    }

    #[tokio::test]
    async fn test_agent_cancellation() {
        let registry = ToolRegistry::with_defaults();
        let backend = MockAgentBackend::from_steps(vec![
            StepResult::Thought("wait".into()),
            StepResult::CallTool {
                tool: "bash".into(),
                input: "sleep 10".into(),
            },
        ]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let config = AgentConfig {
            prompt: "task".into(),
            backend,
            registry,
        };
        let handle = spawn_agent(config, tx);
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AgentEvent::ToolStart { .. }) {
                break;
            }
        }
        handle.cancel();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
