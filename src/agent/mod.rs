pub mod backend;
pub mod http;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod react;
pub mod tool;

use tokio::sync::mpsc;

pub use backend::AgentBackend;
#[cfg(test)]
pub use backend::MockAgentBackend;
pub use ollama::OllamaProvider;
pub use openai::{OpenAiConfig, OpenAiProvider};
pub use provider::{LlmProvider, LlmResponse, ProviderBackend};
pub use react::{AgentEvent, AgentHandle, ReActEngine};
pub use tool::{ToolDefinition, ToolRegistry};

pub enum AnyProvider {
    Ollama(OllamaProvider),
    OpenAi(OpenAiProvider),
}

impl LlmProvider for AnyProvider {
    async fn chat(&self, history: &[backend::AgentMessage], tools: &[ToolDefinition]) -> Result<LlmResponse, String> {
        match self {
            Self::Ollama(p) => p.chat(history, tools).await,
            Self::OpenAi(p) => p.chat(history, tools).await,
        }
    }
}

pub fn default_provider() -> AnyProvider {
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into());
        AnyProvider::OpenAi(OpenAiProvider::new(OpenAiConfig {
            base_url,
            model,
            api_key: Some(key),
        }))
    } else {
        let model = std::env::var("PIRO_MODEL").unwrap_or_else(|_| "ornith-1.5:35b".into());
        if let Ok(host) = std::env::var("OLLAMA_HOST") {
            AnyProvider::Ollama(OllamaProvider::with_base_url(host, model))
        } else {
            AnyProvider::Ollama(OllamaProvider::new(model))
        }
    }
}

pub struct AgentConfig<B> {
    pub prompt: String,
    pub backend: B,
    pub registry: ToolRegistry,
}

pub fn default_backend(registry: &ToolRegistry) -> ProviderBackend<AnyProvider> {
    ProviderBackend::new(default_provider(), registry.definitions())
}

pub fn spawn_agent<B: AgentBackend + 'static>(
    config: AgentConfig<B>,
    agent_tx: mpsc::UnboundedSender<AgentEvent>,
) -> AgentHandle {
    let engine = ReActEngine::new(config.backend, config.registry);
    engine.spawn(config.prompt, agent_tx)
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
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
