pub mod claude;
pub mod http;
pub mod openai;
pub mod sse;

use tokio::sync::mpsc;

use crate::agent::react::AgentEvent;
use crate::agent::tool::ToolDefinition;
use crate::agent::{AgentMessage, AssistantTurn};
use crate::auth::store::default_path;
use claude::ClaudeProvider;

pub use openai::{OpenAiConfig, OpenAiProvider};

pub const DEFAULT_MODEL_SPEC: &str = "ollama/ornith-1.5:35b";

pub struct ChatRequestParams<'a> {
    pub history: &'a [AgentMessage],
    pub tools: &'a [ToolDefinition],
    pub delta_tx: Option<&'a mpsc::UnboundedSender<AgentEvent>>,
}

pub trait LlmProvider: Send + Sync + 'static {
    fn chat_stream<'a>(
        &'a self,
        params: ChatRequestParams<'a>,
    ) -> impl Future<Output = Result<AssistantTurn, String>> + Send + use<'a, Self>;
}

pub enum AnyProvider {
    OpenAi(OpenAiProvider),
    Claude(ClaudeProvider),
}

impl LlmProvider for AnyProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<AssistantTurn, String> {
        match self {
            Self::OpenAi(p) => p.chat_stream(params).await,
            Self::Claude(p) => p.chat_stream(params).await,
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
    let (base_url, key_var) = match provider {
        "ollama" => {
            let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".into());
            (format!("{}/v1", host.trim_end_matches('/')), None)
        }
        "openai" => (
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            Some("OPENAI_API_KEY"),
        ),
        "openrouter" => ("https://openrouter.ai/api/v1".into(), Some("OPENROUTER_API_KEY")),
        "deepseek" => ("https://api.deepseek.com/v1".into(), Some("DEEPSEEK_API_KEY")),
        "groq" => ("https://api.groq.com/openai/v1".into(), Some("GROQ_API_KEY")),
        unsupported => {
            return Err(format!(
                "Unsupported provider '{}'. Supported providers: 'ollama', 'openai', 'openrouter', 'deepseek', 'groq', 'anthropic', 'claude'",
                unsupported
            ));
        }
    };
    let api_key = key_var.and_then(|k| std::env::var(k).ok());
    Ok(OpenAiConfig {
        base_url,
        model: model.into(),
        api_key,
    })
}

pub fn provider_from_spec(spec: &str) -> Result<AnyProvider, String> {
    let parsed = parse_model_spec(spec);
    if parsed.model.is_empty() {
        return Err("Model name cannot be empty".into());
    }
    match parsed.provider.to_lowercase().as_str() {
        "claude" | "claude-code" | "claude-oauth" => {
            let store_path = default_path()?;
            Ok(AnyProvider::Claude(ClaudeProvider::oauth(parsed.model, store_path)))
        }
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty())
                .ok_or("ANTHROPIC_API_KEY is not set; it is required for anthropic/<model>")?;
            Ok(AnyProvider::Claude(ClaudeProvider::api_key(parsed.model, key)))
        }
        provider => {
            let config = resolve_provider_config(provider, parsed.model)?;
            Ok(AnyProvider::OpenAi(OpenAiProvider::new(config)))
        }
    }
}

#[cfg(test)]
pub use mock::MockProvider;

#[cfg(test)]
mod mock {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Replays scripted turns and records each history it was asked to continue.
    #[derive(Default)]
    pub struct MockProvider {
        turns: Vec<AssistantTurn>,
        calls: Arc<AtomicUsize>,
        seen: Arc<Mutex<Vec<Vec<AgentMessage>>>>,
    }

    impl MockProvider {
        pub fn from_turns(turns: Vec<AssistantTurn>) -> Self {
            Self {
                turns,
                ..Default::default()
            }
        }

        pub fn call_counter(&self) -> Arc<AtomicUsize> {
            self.calls.clone()
        }

        pub fn seen_histories(&self) -> Arc<Mutex<Vec<Vec<AgentMessage>>>> {
            self.seen.clone()
        }
    }

    impl LlmProvider for MockProvider {
        async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<AssistantTurn, String> {
            self.seen
                .lock()
                .map_err(|e| e.to_string())?
                .push(params.history.to_vec());
            let i = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.turns.get(i).cloned().unwrap_or_else(|| AssistantTurn {
                text: Some("Done.".into()),
                ..Default::default()
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_prefix_and_defaults_to_ollama() {
        let spec = parse_model_spec("ollama/qwen");
        assert_eq!((spec.provider, spec.model), ("ollama", "qwen"));
        let bare = parse_model_spec("gpt-4o");
        assert_eq!((bare.provider, bare.model), ("ollama", "gpt-4o"));
    }

    #[test]
    fn routes_known_providers_and_rejects_others() {
        for ok in [
            "ollama/ornith-1.5:35b",
            "openai/gpt-4o",
            "deepseek/deepseek-chat",
            "openrouter/auto",
            "groq/llama-3",
            "claude/claude-sonnet-4-6",
        ] {
            assert!(provider_from_spec(ok).is_ok(), "{ok}");
        }
        assert!(provider_from_spec("unsupported/xyz").is_err());
        assert!(provider_from_spec("").is_err());
    }
}
