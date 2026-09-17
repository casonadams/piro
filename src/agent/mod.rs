#![allow(dead_code)]

pub mod backend;
pub mod tool;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::AbortHandle;

#[allow(unused_imports)]
pub use backend::{AgentBackend, AgentMessage, MockAgentBackend, StepResult};
#[allow(unused_imports)]
pub use tool::{BashTool, Tool, ToolRegistry, ToolResult, ToolRun};

use crate::runner::{ProcessHandle, RunnerMsg};

pub const MAX_AGENT_STEPS: usize = 10;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Thought(String),
    ToolStart {
        name: String,
        input: String,
    },
    ToolOutput(String),
    ToolDone {
        success: bool,
        exit_code: Option<i32>,
        duration: Duration,
    },
    Response(String),
    Finished,
    Cancelled,
    Error(String),
}

pub struct AgentHandle {
    abort_handle: AbortHandle,
    active_tool_handle: Arc<Mutex<Option<ProcessHandle>>>,
}

impl AgentHandle {
    pub fn cancel(&self) {
        if let Ok(mut lock) = self.active_tool_handle.try_lock()
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
    let tool_handle_ref = active_tool.clone();

    let task = tokio::spawn(async move {
        let mut history = vec![AgentMessage::User(prompt)];

        for _ in 0..MAX_AGENT_STEPS {
            let next = match backend.next_step(&history).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = agent_tx.send(AgentEvent::Error(e));
                    let _ = agent_tx.send(AgentEvent::Finished);
                    return;
                }
            };

            match next {
                StepResult::Thought(thought) => {
                    let _ = agent_tx.send(AgentEvent::Thought(thought.clone()));
                    history.push(AgentMessage::Thought(thought));
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
                        let (run_tx, mut run_rx) = mpsc::unbounded_channel::<RunnerMsg>();
                        let tool_run = t.run(&input, run_tx);

                        {
                            let mut lock = tool_handle_ref.lock().await;
                            *lock = Some(tool_run.handle);
                        }

                        let forwarder_tx = agent_tx.clone();
                        let fwd_task = tokio::spawn(async move {
                            while let Some(msg) = run_rx.recv().await {
                                if let RunnerMsg::Line(l) = msg {
                                    let _ = forwarder_tx.send(AgentEvent::ToolOutput(l));
                                }
                            }
                        });

                        let res = match tool_run.result.await {
                            Ok(r) => r,
                            Err(_) => {
                                let _ = fwd_task.await;
                                let mut lock = tool_handle_ref.lock().await;
                                *lock = None;
                                let _ = agent_tx.send(AgentEvent::Cancelled);
                                return;
                            }
                        };
                        let _ = fwd_task.await;

                        {
                            let mut lock = tool_handle_ref.lock().await;
                            *lock = None;
                        }

                        let _ = agent_tx.send(AgentEvent::ToolDone {
                            success: res.success,
                            exit_code: res.exit_code,
                            duration: res.duration,
                        });

                        history.push(AgentMessage::Observation {
                            output: res.output,
                            success: res.success,
                        });
                    } else {
                        let err_msg = format!("Tool '{}' not found", name);
                        let _ = agent_tx.send(AgentEvent::Error(err_msg.clone()));
                        history.push(AgentMessage::Observation {
                            output: err_msg,
                            success: false,
                        });
                    }
                }
                StepResult::FinalAnswer(ans) => {
                    let _ = agent_tx.send(AgentEvent::Response(ans));
                    let _ = agent_tx.send(AgentEvent::Finished);
                    return;
                }
            }
        }

        let _ = agent_tx.send(AgentEvent::Response(
            "Max reasoning steps reached without concluding.".into(),
        ));
        let _ = agent_tx.send(AgentEvent::Finished);
    });

    AgentHandle {
        abort_handle: task.abort_handle(),
        active_tool_handle: active_tool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_loop_execution() {
        let registry = ToolRegistry::with_defaults();
        let backend = MockAgentBackend::from_steps(vec![
            StepResult::Thought("Checking files".into()),
            StepResult::CallTool {
                tool: "bash".into(),
                input: "echo 'file1\nfile2'".into(),
            },
            StepResult::FinalAnswer("Found 2 files".into()),
        ]);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let _handle = spawn_agent("count files".into(), backend, registry, tx);

        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let is_finished = matches!(ev, AgentEvent::Finished);
            events.push(ev);
            if is_finished {
                break;
            }
        }

        assert!(events.iter().any(|e| matches!(e, AgentEvent::Thought(_))));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolStart { .. })));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolOutput(_))));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolDone { success: true, .. }))
        );
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Response(_))));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Finished)));
    }

    #[tokio::test]
    async fn test_agent_loop_cancellation() {
        let registry = ToolRegistry::with_defaults();
        let backend = MockAgentBackend::from_steps(vec![
            StepResult::Thought("Long running task".into()),
            StepResult::CallTool {
                tool: "bash".into(),
                input: "sleep 10".into(),
            },
        ]);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = spawn_agent("sleep task".into(), backend, registry, tx);

        // Wait for ToolStart
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AgentEvent::ToolStart { .. }) {
                break;
            }
        }

        // Cancel
        handle.cancel();

        // Check that task halts and does not hang
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
