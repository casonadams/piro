use std::sync::OnceLock;

use serde_json::{Value, json};

use crate::agent::backend::AgentMessage;
use crate::agent::http::{HttpRequest, get_json, post_json};
use crate::agent::provider::{LlmProvider, LlmResponse};
use crate::agent::react::ToolCallRequest;
use crate::agent::tool::ToolDefinition;

/// Fallback when `/api/show` is unreachable or returns no context length.
const OLLAMA_DEFAULT_CTX: u64 = 2048;

pub struct OllamaProvider {
    base_url: String,
    model: String,
    /// Lazily resolved on the first `chat` call.
    num_ctx: OnceLock<u64>,
}

impl OllamaProvider {
    pub fn new(model: String) -> Self {
        Self::with_base_url("http://localhost:11434".into(), model)
    }

    pub fn with_base_url(base_url: String, model: String) -> Self {
        Self {
            base_url,
            model,
            num_ctx: OnceLock::new(),
        }
    }

    /// Queries `/api/show` and extracts the model's native context length.
    /// Returns `None` if Ollama is unreachable or the field is missing.
    async fn fetch_context_length(&self) -> Option<u64> {
        let url = format!("{}/api/show?model={}", self.base_url, self.model);
        let raw = get_json(&url).await.ok()?;
        let val: Value = serde_json::from_str(&raw).ok()?;
        parse_context_length(&val)
    }

    /// Returns the `num_ctx` to use, detecting it once from `/api/show`.
    async fn resolved_num_ctx(&self) -> u64 {
        if let Some(&cached) = self.num_ctx.get() {
            return cached;
        }
        let ctx = self.fetch_context_length().await.unwrap_or(OLLAMA_DEFAULT_CTX);
        // `set` may lose the race in concurrent callers — that's fine.
        let _ = self.num_ctx.set(ctx);
        ctx
    }

    fn format_messages(&self, history: &[AgentMessage]) -> Vec<Value> {
        history
            .iter()
            .filter_map(|msg| match msg {
                AgentMessage::User(u) => Some(json!({ "role": "user", "content": u })),
                AgentMessage::Thought(_) => None,
                AgentMessage::ToolCall { name, input } => Some(json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "function": {
                            "name": name,
                            "arguments": { "command": input }
                        }
                    }]
                })),
                AgentMessage::Observation { output, .. } => Some(json!({ "role": "tool", "content": output })),
                AgentMessage::Response(r) => Some(json!({ "role": "assistant", "content": r })),
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

/// Extracts the model's native context length from an Ollama `/api/show` response.
/// The `model_info` key uses the pattern `<arch>.context_length`.
pub fn parse_context_length(val: &Value) -> Option<u64> {
    let info = val.get("model_info")?.as_object()?;
    info.iter()
        .find(|(k, _)| k.ends_with(".context_length"))
        .and_then(|(_, v)| v.as_u64())
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
    if let Some(err) = val.get("error").and_then(Value::as_str) {
        return Err(format!("Ollama error: {}", err));
    }

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
        let num_ctx = self.resolved_num_ctx().await;
        let url = format!("{}/api/chat", self.base_url);
        let payload = json!({
            "model": self.model,
            "messages": self.format_messages(history),
            "tools": self.format_tools(tools),
            "stream": false,
            "options": { "num_ctx": num_ctx }
        });
        let body_str = payload.to_string();
        let raw = post_json(HttpRequest {
            url: &url,
            body: &body_str,
            auth_token: None,
        })
        .await
        .map_err(|e| {
            if e.contains("Failed to connect") || e.contains("Connection refused") || e.contains("exit 7") {
                format!(
                    "Ollama unreachable at {}. Ensure 'ollama serve' is running.",
                    self.base_url
                )
            } else {
                e
            }
        })?;

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

    #[test]
    fn test_format_ollama_messages_and_tools() {
        let provider = OllamaProvider::new("ornith-1.5:35b".into());
        let history = vec![
            AgentMessage::User("list files".into()),
            AgentMessage::Thought("pondering".into()),
            AgentMessage::ToolCall {
                name: "bash".into(),
                input: "ls".into(),
            },
            AgentMessage::Observation {
                output: "file1".into(),
                success: true,
            },
        ];
        let msgs = provider.format_messages(&history);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["tool_calls"][0]["function"]["name"], "bash");
        assert_eq!(msgs[2]["role"], "tool");

        let tools = vec![ToolDefinition {
            name: "bash",
            description: "shell command",
        }];
        let formatted = provider.format_tools(&tools);
        assert_eq!(formatted.len(), 1);
        assert_eq!(formatted[0]["function"]["name"], "bash");
    }

    #[test]
    fn test_parse_context_length_qwen_moe() {
        let val = json!({
            "model_info": {
                "qwen35moe.context_length": 262144_u64,
                "qwen35moe.block_count": 41
            }
        });
        assert_eq!(parse_context_length(&val), Some(262_144));
    }

    #[test]
    fn test_parse_context_length_llama_family() {
        let val = json!({
            "model_info": {
                "llama.context_length": 131072_u64,
                "llama.block_count": 32
            }
        });
        assert_eq!(parse_context_length(&val), Some(131_072));
    }

    #[test]
    fn test_parse_context_length_missing_returns_none() {
        // No model_info key at all
        assert_eq!(parse_context_length(&json!({})), None);
        // model_info present but no *.context_length key
        assert_eq!(
            parse_context_length(&json!({ "model_info": { "llama.block_count": 32 } })),
            None
        );
    }

    #[test]
    fn test_ollama_default_ctx_used_as_fallback() {
        // Verify the constant is sane and non-zero
        assert!(OLLAMA_DEFAULT_CTX >= 2048);
    }
}
