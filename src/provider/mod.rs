pub mod http;
pub mod openai;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;

use crate::agent::backend::{AgentBackend, AgentMessage, StepResult};
use crate::agent::react::{AgentEvent, ToolCallRequest};
use crate::agent::tool::{ToolDefinition, ToolRegistry};

pub use openai::{OpenAiConfig, OpenAiProvider};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmResponse {
    pub thinking: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub content: Option<String>,
}

pub struct ChatRequestParams<'a> {
    pub history: &'a [AgentMessage],
    pub tools: &'a [ToolDefinition],
    pub delta_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
}

pub trait LlmProvider: Send + Sync + 'static {
    fn chat_stream<'a>(
        &'a self,
        params: ChatRequestParams<'a>,
    ) -> impl std::future::Future<Output = Result<LlmResponse, String>> + Send + use<'a, Self>;
}

pub struct ProviderBackend<P> {
    provider: P,
    tools: Vec<ToolDefinition>,
    pending_tool_call: Option<ToolCallRequest>,
    pending_final_answer: Option<String>,
    delta_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
}

impl<P: LlmProvider> ProviderBackend<P> {
    pub fn new(provider: P, tools: Vec<ToolDefinition>) -> Self {
        Self {
            provider,
            tools,
            pending_tool_call: None,
            pending_final_answer: None,
            delta_tx: None,
        }
    }

    pub fn with_delta_tx(mut self, delta_tx: mpsc::UnboundedSender<AgentEvent>) -> Self {
        self.delta_tx = Some(delta_tx);
        self
    }
}

impl<P: LlmProvider> AgentBackend for ProviderBackend<P> {
    async fn next_step(&mut self, history: &[AgentMessage]) -> Result<StepResult, String> {
        if let Some(call) = self.pending_tool_call.take() {
            return Ok(StepResult::CallTool {
                tool: call.name,
                input: call.input,
            });
        }

        if let Some(ans) = self.pending_final_answer.take() {
            return Ok(StepResult::FinalAnswer(ans));
        }

        let resp = self
            .provider
            .chat_stream(ChatRequestParams {
                history,
                tools: &self.tools,
                delta_tx: self.delta_tx.clone(),
            })
            .await?;

        if let Some(thought) = resp.thinking {
            let mut calls = resp.tool_calls;
            if !calls.is_empty() {
                self.pending_tool_call = Some(calls.remove(0));
            } else if let Some(content) = resp.content {
                self.pending_final_answer = Some(content);
            }
            return Ok(StepResult::Thought(thought));
        }

        if let Some(call) = resp.tool_calls.into_iter().next() {
            return Ok(StepResult::CallTool {
                tool: call.name,
                input: call.input,
            });
        }

        if let Some(content) = resp.content {
            return Ok(StepResult::FinalAnswer(content));
        }

        Ok(StepResult::FinalAnswer("Done.".to_string()))
    }
}

pub enum AnyProvider {
    OpenAi(OpenAiProvider),
}

impl LlmProvider for AnyProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<LlmResponse, String> {
        match self {
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

fn resolve_provider_config(provider: &str, model: &str) -> Result<OpenAiConfig, String> {
    match provider {
        "ollama" => {
            let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".into());
            Ok(OpenAiConfig {
                base_url: format!("{}/v1", host.trim_end_matches('/')),
                model: model.into(),
                api_key: None,
            })
        }
        "openai" => {
            let key = std::env::var("OPENAI_API_KEY").ok();
            let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
            Ok(OpenAiConfig {
                base_url,
                model: model.into(),
                api_key: key,
            })
        }
        "openrouter" => Ok(OpenAiConfig {
            base_url: "https://openrouter.ai/api/v1".into(),
            model: model.into(),
            api_key: std::env::var("OPENROUTER_API_KEY").ok(),
        }),
        "deepseek" => Ok(OpenAiConfig {
            base_url: "https://api.deepseek.com/v1".into(),
            model: model.into(),
            api_key: std::env::var("DEEPSEEK_API_KEY").ok(),
        }),
        "groq" => Ok(OpenAiConfig {
            base_url: "https://api.groq.com/openai/v1".into(),
            model: model.into(),
            api_key: std::env::var("GROQ_API_KEY").ok(),
        }),
        unsupported => Err(format!(
            "Unsupported provider '{}'. Supported providers: 'ollama', 'openai', 'openrouter', 'deepseek', 'groq'",
            unsupported
        )),
    }
}

pub fn provider_from_spec(spec: &str) -> Result<AnyProvider, String> {
    let parsed = parse_model_spec(spec);
    if parsed.model.is_empty() {
        return Err("Model name cannot be empty".into());
    }
    let config = resolve_provider_config(parsed.provider.to_lowercase().as_str(), parsed.model)?;
    Ok(AnyProvider::OpenAi(OpenAiProvider::new(config)))
}

pub fn create_backend(spec: &str, registry: &ToolRegistry) -> Result<ProviderBackend<AnyProvider>, String> {
    let provider = provider_from_spec(spec)?;
    Ok(ProviderBackend::new(provider, registry.definitions()))
}

pub fn default_provider() -> AnyProvider {
    let spec = std::env::var("PIRO_MODEL").unwrap_or_else(|_| "ollama/ornith-1.5:35b".into());
    provider_from_spec(&spec).unwrap_or_else(|_| {
        AnyProvider::OpenAi(OpenAiProvider::new(OpenAiConfig {
            base_url: "http://localhost:11434/v1".into(),
            model: "ornith-1.5:35b".into(),
            api_key: None,
        }))
    })
}

pub fn default_backend(registry: &ToolRegistry) -> ProviderBackend<AnyProvider> {
    ProviderBackend::new(default_provider(), registry.definitions())
}

#[cfg(test)]
#[derive(Default)]
pub struct MockProvider {
    responses: Vec<LlmResponse>,
    idx: AtomicUsize,
}

#[cfg(test)]
impl MockProvider {
    pub fn from_responses(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses,
            idx: AtomicUsize::new(0),
        }
    }
}

#[cfg(test)]
impl LlmProvider for MockProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<LlmResponse, String> {
        let i = self.idx.fetch_add(1, Ordering::SeqCst);
        if let Some(resp) = self.responses.get(i).cloned() {
            return Ok(resp);
        }

        let prompt = params
            .history
            .iter()
            .find_map(|m| match m {
                AgentMessage::User(u) => Some(u.as_str()),
                _ => None,
            })
            .unwrap_or("");

        let has_thought = params.history.iter().any(|m| matches!(m, AgentMessage::Thought(_)));
        if !has_thought {
            return Ok(LlmResponse {
                thinking: Some("Reasoning about task".into()),
                tool_calls: vec![ToolCallRequest {
                    name: "bash".into(),
                    input: "echo demo".into(),
                }],
                content: None,
            });
        }

        Ok(LlmResponse {
            thinking: None,
            tool_calls: Vec::new(),
            content: Some(format!("Completed: {}", prompt)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_and_backend() {
        let provider = MockProvider::from_responses(vec![
            LlmResponse {
                thinking: Some("Plan ahead".into()),
                tool_calls: vec![ToolCallRequest {
                    name: "bash".into(),
                    input: "echo 1".into(),
                }],
                content: None,
            },
            LlmResponse {
                thinking: None,
                tool_calls: Vec::new(),
                content: Some("Result 1".into()),
            },
        ]);

        let mut backend = ProviderBackend::new(provider, Vec::new());
        let step1 = backend.next_step(&[]).await.unwrap();
        assert_eq!(step1, StepResult::Thought("Plan ahead".into()));

        let step2 = backend.next_step(&[]).await.unwrap();
        assert!(matches!(step2, StepResult::CallTool { .. }));
    }

    #[tokio::test]
    async fn test_engine_with_provider_backend() {
        let registry = ToolRegistry::with_defaults();
        let provider = MockProvider::from_responses(vec![
            LlmResponse {
                thinking: Some("Running bash echo".into()),
                tool_calls: vec![ToolCallRequest {
                    name: "bash".into(),
                    input: "echo 'provider test'".into(),
                }],
                content: None,
            },
            LlmResponse {
                thinking: None,
                tool_calls: Vec::new(),
                content: Some("Provider test finished".into()),
            },
        ]);
        let backend = ProviderBackend::new(provider, registry.definitions());
        let engine = crate::agent::ReActEngine::new(backend, registry);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _handle = engine.spawn("run provider".into(), tx);

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
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Response(_))));
    }

    #[test]
    fn test_parse_model_spec() {
        let spec = parse_model_spec("ollama/qwen");
        assert_eq!(spec.provider, "ollama");
        assert_eq!(spec.model, "qwen");

        let default_prov = parse_model_spec("gpt-4o");
        assert_eq!(default_prov.provider, "ollama");
        assert_eq!(default_prov.model, "gpt-4o");
    }

    #[test]
    fn test_provider_from_spec_routing() {
        assert!(provider_from_spec("ollama/ornith-1.5:35b").is_ok());
        assert!(provider_from_spec("openai/gpt-4o").is_ok());
        assert!(provider_from_spec("deepseek/deepseek-chat").is_ok());
        assert!(provider_from_spec("openrouter/auto").is_ok());
        assert!(provider_from_spec("groq/llama-3").is_ok());
        assert!(provider_from_spec("unsupported/xyz").is_err());
        assert!(provider_from_spec("").is_err());
    }
}
