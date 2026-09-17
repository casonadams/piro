use std::sync::OnceLock;

use futures_util::StreamExt;
use reqwest::Response;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::agent::backend::AgentMessage;
use crate::agent::react::{AgentEvent, ToolCallRequest};
use crate::agent::tool::ToolDefinition;
use crate::provider::http::{HttpRequest, post_json, post_request};
use crate::provider::{ChatRequestParams, LlmProvider, LlmResponse};

/// Fallback when `/api/show` is unreachable or returns no context length.
const OLLAMA_DEFAULT_CTX: u64 = 8192;

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

    /// Queries `/api/show` via POST and extracts the model's native context length.
    /// Returns `None` if Ollama is unreachable or the field is missing.
    async fn fetch_context_length(&self) -> Option<u64> {
        let url = format!("{}/api/show", self.base_url);
        let body = json!({ "model": self.model }).to_string();
        let raw = post_json(HttpRequest {
            url: &url,
            body: &body,
            auth_token: None,
        })
        .await
        .ok()?;
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
/// Checks `model_info.<arch>.context_length` first, then falls back to `details.context_length`.
pub fn parse_context_length(val: &Value) -> Option<u64> {
    if let Some(ctx) = val.get("model_info").and_then(Value::as_object).and_then(|info| {
        info.iter()
            .find(|(k, _)| k.ends_with(".context_length"))
            .and_then(|(_, v)| v.as_u64())
    }) {
        return Some(ctx);
    }
    val.get("details")
        .and_then(Value::as_object)
        .and_then(|d| d.get("context_length"))
        .and_then(Value::as_u64)
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

#[derive(Default)]
pub struct OllamaStreamAccumulator {
    pub thinking: String,
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub done: bool,
    pub done_reason: Option<String>,
}

impl OllamaStreamAccumulator {
    pub fn process_chunk(
        &mut self,
        val: &Value,
        delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<(), String> {
        if let Some(err) = val.get("error").and_then(Value::as_str) {
            return Err(format!("Ollama error: {}", err));
        }
        if let Some(msg) = val.get("message") {
            if let Some(th) = msg.get("thinking").and_then(Value::as_str)
                && !th.is_empty()
            {
                self.thinking.push_str(th);
                if let Some(tx) = delta_tx {
                    let _ = tx.send(AgentEvent::ThoughtDelta(th.to_string()));
                }
            }
            if let Some(c) = msg.get("content").and_then(Value::as_str)
                && !c.is_empty()
            {
                self.content.push_str(c);
                if let Some(tx) = delta_tx {
                    let _ = tx.send(AgentEvent::ResponseDelta(c.to_string()));
                }
            }
            let calls = parse_tool_calls(msg);
            if !calls.is_empty() {
                self.tool_calls.extend(calls);
            }
        }
        if let Some(done) = val.get("done").and_then(Value::as_bool) {
            self.done = done;
        }
        if let Some(reason) = val.get("done_reason").and_then(Value::as_str) {
            self.done_reason = Some(reason.to_string());
        }
        Ok(())
    }

    pub fn finish(self) -> LlmResponse {
        let thinking = if self.thinking.trim().is_empty() {
            None
        } else {
            Some(self.thinking)
        };
        let content = if self.content.trim().is_empty() {
            None
        } else {
            Some(self.content)
        };
        LlmResponse {
            thinking,
            tool_calls: self.tool_calls,
            content,
        }
    }
}

#[cfg(test)]
fn parse_ollama_response(val: Value) -> Result<LlmResponse, String> {
    let mut acc = OllamaStreamAccumulator::default();
    acc.process_chunk(&val, None)?;
    Ok(acc.finish())
}

async fn read_stream_response(
    res: Response,
    delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
) -> Result<LlmResponse, String> {
    let mut stream = res.bytes_stream();
    let mut acc = OllamaStreamAccumulator::default();
    let mut line_buf = String::new();

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| format!("Failed to read stream chunk: {}", e))?;
        let text = String::from_utf8_lossy(&chunk);
        line_buf.push_str(&text);

        while let Some(pos) = line_buf.find('\n') {
            let line = line_buf[..pos].trim().to_string();
            line_buf.drain(..=pos);
            if !line.is_empty() {
                let val: Value =
                    serde_json::from_str(&line).map_err(|e| format!("Failed to parse Ollama stream JSON: {}", e))?;
                acc.process_chunk(&val, delta_tx)?;
            }
        }
    }

    let remainder = line_buf.trim();
    if !remainder.is_empty() {
        let val: Value =
            serde_json::from_str(remainder).map_err(|e| format!("Failed to parse Ollama stream JSON: {}", e))?;
        acc.process_chunk(&val, delta_tx)?;
    }

    Ok(acc.finish())
}

impl LlmProvider for OllamaProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<LlmResponse, String> {
        let num_ctx = self.resolved_num_ctx().await;
        let url = format!("{}/api/chat", self.base_url);
        let payload = json!({
            "model": self.model,
            "messages": self.format_messages(params.history),
            "tools": self.format_tools(params.tools),
            "stream": true,
            "options": { "num_ctx": num_ctx }
        });
        let body_str = payload.to_string();
        let res = post_request(HttpRequest {
            url: &url,
            body: &body_str,
            auth_token: None,
        })
        .await
        .map_err(|e| {
            if e.contains("connect") || e.contains("refused") {
                format!(
                    "Ollama unreachable at {}. Ensure 'ollama serve' is running.",
                    self.base_url
                )
            } else {
                e
            }
        })?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("Ollama HTTP error {}: {}", status, body));
        }

        read_stream_response(res, params.delta_tx.as_ref()).await
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
    fn test_parse_context_length_details_fallback() {
        let val = json!({
            "details": {
                "context_length": 65536_u64
            }
        });
        assert_eq!(parse_context_length(&val), Some(65_536));
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

    #[test]
    fn test_stream_accumulator_multi_chunk() {
        let mut acc = OllamaStreamAccumulator::default();
        acc.process_chunk(
            &json!({
                "message": { "role": "assistant", "content": "", "thinking": "Let " }
            }),
            None,
        )
        .unwrap();
        acc.process_chunk(
            &json!({
                "message": { "role": "assistant", "content": "", "thinking": "me check" }
            }),
            None,
        )
        .unwrap();
        acc.process_chunk(
            &json!({
                "message": { "role": "assistant", "content": "Done", "thinking": "" }
            }),
            None,
        )
        .unwrap();
        acc.process_chunk(
            &json!({
                "done": true,
                "done_reason": "stop"
            }),
            None,
        )
        .unwrap();

        let res = acc.finish();
        assert_eq!(res.thinking.as_deref(), Some("Let me check"));
        assert_eq!(res.content.as_deref(), Some("Done"));
        assert!(res.tool_calls.is_empty());
    }

    #[test]
    fn test_stream_accumulator_with_delta_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut acc = OllamaStreamAccumulator::default();
        acc.process_chunk(
            &json!({
                "message": { "role": "assistant", "content": "", "thinking": "Pondering" }
            }),
            Some(&tx),
        )
        .unwrap();
        acc.process_chunk(
            &json!({
                "message": { "role": "assistant", "content": "Result", "thinking": "" }
            }),
            Some(&tx),
        )
        .unwrap();

        let ev1 = rx.try_recv().unwrap();
        let ev2 = rx.try_recv().unwrap();
        assert!(matches!(ev1, AgentEvent::ThoughtDelta(t) if t == "Pondering"));
        assert!(matches!(ev2, AgentEvent::ResponseDelta(r) if r == "Result"));
    }

    #[test]
    fn test_stream_accumulator_tool_call() {
        let mut acc = OllamaStreamAccumulator::default();
        acc.process_chunk(
            &json!({
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "function": {
                            "name": "bash",
                            "arguments": { "command": "git status" }
                        }
                    }]
                },
                "done": true
            }),
            None,
        )
        .unwrap();

        let res = acc.finish();
        assert_eq!(res.tool_calls.len(), 1);
        assert_eq!(res.tool_calls[0].name, "bash");
        assert_eq!(res.tool_calls[0].input, "git status");
    }

    #[test]
    fn test_stream_accumulator_error() {
        let mut acc = OllamaStreamAccumulator::default();
        let err = acc.process_chunk(
            &json!({
                "error": "model not found"
            }),
            None,
        );
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), "Ollama error: model not found");
    }
}
