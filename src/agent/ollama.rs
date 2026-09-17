use serde_json::{Value, json};

use crate::agent::backend::AgentMessage;
use crate::agent::http::{HttpRequest, post_json};
use crate::agent::provider::{LlmProvider, LlmResponse};
use crate::agent::react::ToolCallRequest;
use crate::agent::tool::ToolDefinition;

pub struct OllamaProvider {
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(model: String) -> Self {
        Self::with_base_url("http://localhost:11434".into(), model)
    }

    pub fn with_base_url(base_url: String, model: String) -> Self {
        Self { base_url, model }
    }

    fn format_messages(&self, history: &[AgentMessage]) -> Vec<Value> {
        history
            .iter()
            .map(|msg| match msg {
                AgentMessage::User(u) => json!({ "role": "user", "content": u }),
                AgentMessage::Thought(t) => {
                    json!({ "role": "assistant", "content": "", "thinking": t })
                }
                AgentMessage::ToolCall { name, input } => {
                    json!({
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "function": {
                                "name": name,
                                "arguments": { "command": input }
                            }
                        }]
                    })
                }
                AgentMessage::Observation { output, .. } => {
                    json!({ "role": "tool", "content": output })
                }
                AgentMessage::Response(r) => json!({ "role": "assistant", "content": r }),
            })
            .collect()
    }

    fn format_tools(&self, tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "command": {
                                    "type": "string",
                                    "description": "The command or arguments to execute"
                                }
                            },
                            "required": ["command"]
                        }
                    }
                })
            })
            .collect()
    }
}

fn parse_tool_calls(val: &Value) -> Vec<ToolCallRequest> {
    let Some(calls) = val.get("tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };
    calls
        .iter()
        .filter_map(|c| {
            let func = c.get("function")?;
            let name = func.get("name")?.as_str()?.to_string();
            let args = func.get("arguments")?;
            let input = if let Some(cmd) = args.get("command").and_then(Value::as_str) {
                cmd.to_string()
            } else if let Some(s) = args.as_str() {
                s.to_string()
            } else {
                args.to_string()
            };
            Some(ToolCallRequest { name, input })
        })
        .collect()
}

fn parse_ollama_response(val: Value) -> Result<LlmResponse, String> {
    let msg = val
        .get("message")
        .ok_or_else(|| "Ollama response missing 'message'".to_string())?;

    let thinking = msg
        .get("thinking")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    let tool_calls = parse_tool_calls(msg);
    let content = msg
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    Ok(LlmResponse {
        thinking,
        tool_calls,
        content,
    })
}

impl LlmProvider for OllamaProvider {
    async fn chat(&self, history: &[AgentMessage], tools: &[ToolDefinition]) -> Result<LlmResponse, String> {
        let url = format!("{}/api/chat", self.base_url);
        let payload = json!({
            "model": self.model,
            "messages": self.format_messages(history),
            "tools": self.format_tools(tools),
            "stream": false
        });
        let body_str = payload.to_string();
        let raw = post_json(HttpRequest {
            url: &url,
            body: &body_str,
            auth_token: None,
        })
        .await?;

        let body: Value = serde_json::from_str(&raw).map_err(|e| format!("Failed to parse Ollama JSON: {}", e))?;

        parse_ollama_response(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ollama_thinking_and_tools() {
        let raw = json!({
            "message": {
                "role": "assistant",
                "content": "",
                "thinking": "Pondering the task",
                "tool_calls": [{
                    "function": {
                        "name": "bash",
                        "arguments": { "command": "ls -1" }
                    }
                }]
            }
        });
        let res = parse_ollama_response(raw).unwrap();
        assert_eq!(res.thinking.as_deref(), Some("Pondering the task"));
        assert_eq!(res.tool_calls.len(), 1);
        assert_eq!(res.tool_calls[0].name, "bash");
        assert_eq!(res.tool_calls[0].input, "ls -1");
        assert_eq!(res.content, None);
    }

    #[test]
    fn test_parse_ollama_final_content() {
        let raw = json!({
            "message": {
                "role": "assistant",
                "content": "Found 5 files in the project.",
                "thinking": ""
            }
        });
        let res = parse_ollama_response(raw).unwrap();
        assert_eq!(res.thinking, None);
        assert!(res.tool_calls.is_empty());
        assert_eq!(res.content.as_deref(), Some("Found 5 files in the project."));
    }

    #[tokio::test]
    async fn test_live_ollama_query_if_running() {
        let provider = OllamaProvider::new("ornith-1.5:35b".into());
        let history = vec![AgentMessage::User("Respond in one word: ready?".into())];
        if let Ok(resp) = provider.chat(&history, &[]).await {
            assert!(resp.content.is_some() || resp.thinking.is_some());
        }
    }
}
