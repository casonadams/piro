#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::agent::backend::{AgentBackend, AgentMessage, StepResult};
use crate::agent::react::ToolCallRequest;
use crate::agent::tool::ToolDefinition;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmResponse {
    pub thinking: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub content: Option<String>,
}

pub trait LlmProvider: Send + Sync + 'static {
    fn chat(
        &self,
        history: &[AgentMessage],
        tools: &[ToolDefinition],
    ) -> impl std::future::Future<Output = Result<LlmResponse, String>> + Send;
}

pub struct ProviderBackend<P> {
    provider: P,
    tools: Vec<ToolDefinition>,
    pending_tool_call: Option<ToolCallRequest>,
}

impl<P: LlmProvider> ProviderBackend<P> {
    pub fn new(provider: P, tools: Vec<ToolDefinition>) -> Self {
        Self {
            provider,
            tools,
            pending_tool_call: None,
        }
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

        let resp = self.provider.chat(history, &self.tools).await?;

        if let Some(thought) = resp.thinking {
            let mut calls = resp.tool_calls;
            if !calls.is_empty() {
                self.pending_tool_call = Some(calls.remove(0));
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
    async fn chat(&self, history: &[AgentMessage], _tools: &[ToolDefinition]) -> Result<LlmResponse, String> {
        let i = self.idx.fetch_add(1, Ordering::SeqCst);
        if let Some(resp) = self.responses.get(i).cloned() {
            return Ok(resp);
        }

        let prompt = history
            .iter()
            .find_map(|m| match m {
                AgentMessage::User(u) => Some(u.as_str()),
                _ => None,
            })
            .unwrap_or("");

        let has_thought = history.iter().any(|m| matches!(m, AgentMessage::Thought(_)));
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
        let registry = crate::agent::tool::ToolRegistry::with_defaults();
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
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _handle = engine.spawn("run provider".into(), tx);

        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let is_fin = matches!(ev, crate::agent::AgentEvent::Finished);
            events.push(ev);
            if is_fin {
                break;
            }
        }
        assert!(events.iter().any(|e| matches!(e, crate::agent::AgentEvent::Thought(_))));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, crate::agent::AgentEvent::ToolStart { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, crate::agent::AgentEvent::Response(_)))
        );
    }
}
