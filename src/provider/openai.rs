use std::collections::BTreeMap;

use futures_util::StreamExt;
use reqwest::Response;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::agent::backend::AgentMessage;
use crate::agent::react::{AgentEvent, ToolCallRequest};
use crate::agent::tool::ToolDefinition;
use crate::provider::http::{HttpRequest, post_request};
use crate::provider::{ChatRequestParams, LlmProvider, LlmResponse};

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

pub struct OpenAiProvider {
    config: OpenAiConfig,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig) -> Self {
        Self { config }
    }

    fn format_messages(&self, history: &[AgentMessage]) -> Vec<Value> {
        history
            .iter()
            .filter_map(|msg| match msg {
                AgentMessage::User(u) => Some(json!({ "role": "user", "content": u })),
                AgentMessage::Thought(_) => None,
                AgentMessage::ToolCall { name, input } => Some(json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": format!("call_{}", name),
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": json!({ "command": input }).to_string()
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
                                    "description": "Command or arguments"
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

#[derive(Default)]
struct ToolCallAccumulator {
    name: String,
    arguments: String,
}

#[derive(Default)]
pub struct SseChatAccumulator {
    pub thinking: String,
    pub content: String,
    tool_calls: BTreeMap<usize, ToolCallAccumulator>,
}

impl SseChatAccumulator {
    pub fn process_line(
        &mut self,
        line: &str,
        delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<bool, String> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            return Ok(false);
        }
        let Some(payload) = trimmed.strip_prefix("data:") else {
            return Ok(false);
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            return Ok(true);
        }

        let val: Value = serde_json::from_str(payload).map_err(|e| format!("Failed to parse SSE JSON: {}", e))?;

        if let Some(err) = val.get("error").and_then(Value::as_str) {
            return Err(format!("OpenAI error: {}", err));
        }

        self.process_chunk(&val, delta_tx);
        Ok(false)
    }

    fn process_chunk(&mut self, val: &Value, delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>) {
        let Some(choice) = val.get("choices").and_then(|c| c.get(0)) else {
            return;
        };
        let Some(delta) = choice.get("delta") else {
            return;
        };

        let reasoning = delta
            .get("reasoning")
            .or_else(|| delta.get("reasoning_content"))
            .and_then(Value::as_str);

        if let Some(th) = reasoning
            && !th.is_empty()
        {
            self.thinking.push_str(th);
            if let Some(tx) = delta_tx {
                let _ = tx.send(AgentEvent::ThoughtDelta(th.to_string()));
            }
        }

        if let Some(c) = delta.get("content").and_then(Value::as_str)
            && !c.is_empty()
        {
            self.content.push_str(c);
            if let Some(tx) = delta_tx {
                let _ = tx.send(AgentEvent::ResponseDelta(c.to_string()));
            }
        }

        if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
            self.accumulate_tool_calls(tcs);
        }
    }

    fn accumulate_tool_calls(&mut self, tcs: &[Value]) {
        for tc in tcs {
            let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let entry = self.tool_calls.entry(idx).or_default();
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(Value::as_str) {
                    entry.name.push_str(name);
                }
                if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                    entry.arguments.push_str(args);
                }
            }
        }
    }

    pub fn finish(self) -> LlmResponse {
        let (thinking, content) = finalize_thinking_and_content(self.thinking, self.content);
        let tool_calls = self
            .tool_calls
            .into_values()
            .filter(|tc| !tc.name.is_empty())
            .map(|tc| ToolCallRequest {
                name: tc.name,
                input: parse_tool_arg_command(&tc.arguments),
            })
            .collect();

        LlmResponse {
            thinking,
            tool_calls,
            content,
        }
    }
}

fn parse_tool_arg_command(args: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(args)
        && let Some(cmd) = val.get("command").and_then(Value::as_str)
    {
        return cmd.to_string();
    }
    args.to_string()
}

fn finalize_thinking_and_content(thinking: String, content: String) -> (Option<String>, Option<String>) {
    let th_opt = if thinking.trim().is_empty() {
        None
    } else {
        Some(thinking)
    };
    if let Some(th) = th_opt {
        let c_opt = if content.trim().is_empty() { None } else { Some(content) };
        return (Some(th), c_opt);
    }
    if let Some(start) = content.find("<think>")
        && let Some(end) = content.find("</think>")
    {
        let thought = content[start + 7..end].trim().to_string();
        let remainder = content[end + 8..].trim().to_string();
        let rem_opt = if remainder.is_empty() { None } else { Some(remainder) };
        return (Some(thought), rem_opt);
    }
    let c_opt = if content.trim().is_empty() { None } else { Some(content) };
    (None, c_opt)
}

async fn read_sse_stream(
    res: Response,
    delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
) -> Result<LlmResponse, String> {
    let mut stream = res.bytes_stream();
    let mut acc = SseChatAccumulator::default();
    let mut line_buf = String::new();

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| format!("Failed to read stream chunk: {}", e))?;
        let text = String::from_utf8_lossy(&chunk);
        line_buf.push_str(&text);

        while let Some(pos) = line_buf.find('\n') {
            let line = line_buf[..pos].trim().to_string();
            line_buf.drain(..=pos);
            if !line.is_empty() && acc.process_line(&line, delta_tx)? {
                return Ok(acc.finish());
            }
        }
    }

    let remainder = line_buf.trim();
    if !remainder.is_empty() {
        let _ = acc.process_line(remainder, delta_tx)?;
    }

    Ok(acc.finish())
}

impl LlmProvider for OpenAiProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<LlmResponse, String> {
        let base = self.config.base_url.trim_end_matches('/');
        let url = if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        };
        let payload = json!({
            "model": self.config.model,
            "messages": self.format_messages(params.history),
            "tools": self.format_tools(params.tools),
            "stream": true,
        });
        let body_str = payload.to_string();
        let res = post_request(HttpRequest {
            url: &url,
            body: &body_str,
            auth_token: self.config.api_key.as_deref(),
        })
        .await
        .map_err(|e| {
            if e.contains("connect") || e.contains("refused") {
                format!(
                    "Provider unreachable at {}. Ensure service is running.",
                    self.config.base_url
                )
            } else {
                e
            }
        })?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("HTTP error {}: {}", status, body));
        }

        read_sse_stream(res, params.delta_tx.as_ref()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_accumulator_ollama_reasoning_and_content() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut acc = SseChatAccumulator::default();

        let line1 = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"Pondering\"}}]}";
        assert!(!acc.process_line(line1, Some(&tx)).unwrap());

        let line2 = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello world\"}}]}";
        assert!(!acc.process_line(line2, Some(&tx)).unwrap());

        let line3 = "data: [DONE]";
        assert!(acc.process_line(line3, Some(&tx)).unwrap());

        let res = acc.finish();
        assert_eq!(res.thinking.as_deref(), Some("Pondering"));
        assert_eq!(res.content.as_deref(), Some("Hello world"));

        let ev1 = rx.try_recv().unwrap();
        let ev2 = rx.try_recv().unwrap();
        assert!(matches!(ev1, AgentEvent::ThoughtDelta(t) if t == "Pondering"));
        assert!(matches!(ev2, AgentEvent::ResponseDelta(r) if r == "Hello world"));
    }

    #[test]
    fn test_sse_accumulator_deepseek_reasoning_content() {
        let mut acc = SseChatAccumulator::default();
        let line = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"Deep reasoning\"}}]}";
        assert!(!acc.process_line(line, None).unwrap());
        let res = acc.finish();
        assert_eq!(res.thinking.as_deref(), Some("Deep reasoning"));
    }

    #[test]
    fn test_sse_accumulator_split_tool_calls() {
        let mut acc = SseChatAccumulator::default();

        let l1 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"bash","arguments":"{\"command\": "}}]}}]}"#;
        let l2 =
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"echo 42\"}"}}]}}]}"#;
        let l3 = "data: [DONE]";

        assert!(!acc.process_line(l1, None).unwrap());
        assert!(!acc.process_line(l2, None).unwrap());
        assert!(acc.process_line(l3, None).unwrap());

        let res = acc.finish();
        assert_eq!(res.tool_calls.len(), 1);
        assert_eq!(res.tool_calls[0].name, "bash");
        assert_eq!(res.tool_calls[0].input, "echo 42");
    }

    #[test]
    fn test_sse_accumulator_inline_think_tags() {
        let mut acc = SseChatAccumulator::default();
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"<think>Inline plan</think>Final answer\"}}]}";
        assert!(!acc.process_line(line, None).unwrap());
        let res = acc.finish();
        assert_eq!(res.thinking.as_deref(), Some("Inline plan"));
        assert_eq!(res.content.as_deref(), Some("Final answer"));
    }
}
