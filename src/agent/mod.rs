pub mod backend;
pub mod http;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod react;
pub mod tool;

use tokio::sync::mpsc;

#[cfg(test)]
pub use backend::MockAgentBackend;
pub use backend::{AgentBackend, AgentMessage};
pub use ollama::OllamaProvider;
pub use openai::{OpenAiConfig, OpenAiProvider};
pub use provider::{ChatRequestParams, LlmProvider, LlmResponse, ProviderBackend};
pub use react::{AgentEvent, AgentHandle, EngineSpawnOptions, ReActEngine};
pub use tool::ToolRegistry;

pub enum AnyProvider {
    Ollama(OllamaProvider),
    OpenAi(OpenAiProvider),
}

impl LlmProvider for AnyProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<LlmResponse, String> {
        match self {
            Self::Ollama(p) => p.chat_stream(params).await,
            Self::OpenAi(p) => p.chat_stream(params).await,
        }
    }
}

pub struct ModelSpec<'a> {
    pub provider: &'a str,
    pub model: &'a str,
}

pub fn parse_model_spec(spec: &str) -> ModelSpec<'_> {
    if let Some((prov, model)) = spec.split_once('/') {
        ModelSpec {
            provider: prov.trim(),
            model: model.trim(),
        }
    } else {
        ModelSpec {
            provider: "ollama",
            model: spec.trim(),
        }
    }
}

pub fn provider_from_spec(spec: &str) -> Result<AnyProvider, String> {
    let parsed = parse_model_spec(spec);
    if parsed.model.is_empty() {
        return Err("Model name cannot be empty".into());
    }
    match parsed.provider.to_lowercase().as_str() {
        "ollama" => {
            if let Ok(host) = std::env::var("OLLAMA_HOST") {
                Ok(AnyProvider::Ollama(OllamaProvider::with_base_url(
                    host,
                    parsed.model.into(),
                )))
            } else {
                Ok(AnyProvider::Ollama(OllamaProvider::new(parsed.model.into())))
            }
        }
        "openai" => {
            let key = std::env::var("OPENAI_API_KEY").ok();
            let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into());
            Ok(AnyProvider::OpenAi(OpenAiProvider::new(OpenAiConfig {
                base_url,
                model: parsed.model.into(),
                api_key: key,
            })))
        }
        unsupported => Err(format!(
            "Unsupported provider '{}'. Supported providers: 'ollama', 'openai'",
            unsupported
        )),
    }
}

pub fn create_backend(spec: &str, registry: &ToolRegistry) -> Result<ProviderBackend<AnyProvider>, String> {
    let provider = provider_from_spec(spec)?;
    Ok(ProviderBackend::new(provider, registry.definitions()))
}

pub fn default_provider() -> AnyProvider {
    let spec = std::env::var("PIRO_MODEL").unwrap_or_else(|_| "ollama/ornith-1.5:35b".into());
    provider_from_spec(&spec).unwrap_or_else(|_| AnyProvider::Ollama(OllamaProvider::new("ornith-1.5:35b".into())))
}

pub struct AgentConfig<B> {
    pub prompt: String,
    pub backend: B,
    pub registry: ToolRegistry,
    pub history: Vec<AgentMessage>,
}

pub fn default_backend(registry: &ToolRegistry) -> ProviderBackend<AnyProvider> {
    ProviderBackend::new(default_provider(), registry.definitions())
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
        let (tx, mut rx) = mpsc::unbounded_channel();
        let config = AgentConfig {
            prompt: "task".into(),
            backend,
            registry,
            history: Vec::new(),
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
