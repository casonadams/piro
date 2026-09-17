pub mod backend;
pub mod react;
pub mod tool;

use tokio::sync::mpsc;

#[cfg(test)]
pub use backend::MockAgentBackend;
pub use backend::{AgentBackend, AgentMessage};
pub use react::{AgentEvent, AgentHandle, EngineSpawnOptions, ReActEngine};
pub use tool::ToolRegistry;

pub struct AgentConfig<B> {
    pub prompt: String,
    pub backend: B,
    pub registry: ToolRegistry,
    pub history: Vec<AgentMessage>,
}

pub fn spawn_agent<B: AgentBackend + 'static>(
    config: AgentConfig<B>,
    agent_tx: mpsc::UnboundedSender<AgentEvent>,
) -> AgentHandle {
    let engine = ReActEngine::new(config.backend, config.registry);
    engine.spawn_with_options(EngineSpawnOptions {
        prompt: config.prompt,
        history: config.history,
        agent_tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::StepResult;

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
            history: Vec::new(),
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
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = AgentConfig {
            prompt: "task".into(),
            backend,
            registry,
            history: Vec::new(),
        };
        let handle = spawn_agent(config, tx);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        handle.cancel();
    }
}
