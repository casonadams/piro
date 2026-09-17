pub mod backend;
pub mod tool;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::AbortHandle;

pub use backend::{AgentBackend, AgentMessage, MockAgentBackend, StepResult};
pub use tool::{Tool, ToolRegistry};

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

pub fn spawn_agent<B: AgentBackend + 'static>(
    prompt: String,
    mut backend: B,
    registry: ToolRegistry,
    agent_tx: mpsc::UnboundedSender<AgentEvent>,
) -> AgentHandle {
    let active_tool = Arc::new(Mutex::new(None));
    let tool_ref = active_tool.clone();

    let task = tokio::spawn(async move {
        let _tools = registry.format_descriptions();
        let mut history = vec![AgentMessage::User(prompt)];
        for _ in 0..MAX_AGENT_STEPS {
            let next = match backend.next_step(&history).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = agent_tx.send(AgentEvent::Error(e));
                    break;
                }
            };
            match next {
                StepResult::Thought(t) => {
                    let _ = agent_tx.send(AgentEvent::Thought(t.clone()));
                    history.push(AgentMessage::Thought(t));
                }
                StepResult::CallTool { tool: name, input } => {
                    let _ = agent_tx.send(AgentEvent::ToolStart {
                        name: name.clone(),
                        input: input.clone(),
                    });
                    history.push(AgentMessage::ToolCall {
                        name: name.clone(),
                        input: input.clone(),
                    });
                    if let Some(t) = registry.get(&name) {
                        let (line_tx, mut line_rx) = mpsc::unbounded_channel();
                        let tool_run = t.run(&input, line_tx);
                        *tool_ref.lock().await = Some(tool_run.handle);
                        let fwd = agent_tx.clone();
                        let fwd_task = tokio::spawn(async move {
                            while let Some(l) = line_rx.recv().await {
                                let _ = fwd.send(AgentEvent::ToolOutput(l));
                            }
                        });
                        let Ok(res) = tool_run.result.await else {
                            let _ = fwd_task.await;
                            *tool_ref.lock().await = None;
                            let _ = agent_tx.send(AgentEvent::Cancelled);
                            return;
                        };
                        let _ = fwd_task.await;
                        *tool_ref.lock().await = None;
                        let _ = agent_tx.send(AgentEvent::ToolDone {
                            exit_code: res.exit_code,
                            duration: res.duration,
                        });
                        history.push(AgentMessage::Observation {
                            output: res.output,
                            success: res.success,
                        });
                    } else {
                        let err = format!("Tool '{}' not found", name);
                        let _ = agent_tx.send(AgentEvent::Error(err.clone()));
                        history.push(AgentMessage::Observation {
                            output: err,
                            success: false,
                        });
                    }
                }
                StepResult::FinalAnswer(ans) => {
                    history.push(AgentMessage::Response(ans.clone()));
                    let _ = agent_tx.send(AgentEvent::Response(ans));
                    break;
                }
            }
        }
        let _ = agent_tx.send(AgentEvent::Finished);
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
        let _handle = spawn_agent("count files".into(), backend, registry, tx);

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
        let handle = spawn_agent("task".into(), backend, registry, tx);
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AgentEvent::ToolStart { .. }) {
                break;
            }
        }
        handle.cancel();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
