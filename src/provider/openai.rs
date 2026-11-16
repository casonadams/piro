use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::agent::react::AgentEvent;
use crate::agent::tool::ToolDefinition;
use crate::agent::{AgentMessage, AssistantTurn, Thinking, ToolCall};
use crate::provider::http::{HttpRequest, post_request};
use crate::provider::sse::{Flow, data_payload, for_each_line};
use crate::provider::{ChatRequestParams, LlmProvider};

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

pub struct OpenAiProvider {
    config: OpenAiConfig,
}

fn format_message(msg: &AgentMessage) -> Value {
    match msg {
        AgentMessage::User(u) => json!({ "role": "user", "content": u }),
        AgentMessage::Assistant(turn) => {
            let mut m = json!({ "role": "assistant", "content": turn.text });
            if !turn.tool_calls.is_empty() {
                let calls: Vec<Value> = turn.tool_calls.iter().map(format_tool_call).collect();
                m["tool_calls"] = Value::Array(calls);
            }
            m
        }
        AgentMessage::ToolResult { call_id, output, .. } => {
            json!({ "role": "tool", "tool_call_id": call_id, "content": output })
        }
    }
}

fn format_tool_call(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": json!({ "command": call.input }).to_string()
        }
    })
}

fn format_tools(tools: &[ToolDefinition]) -> Vec<Value> {
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
                            "command": { "type": "string", "description": "Command or arguments" }
                        },
                        "required": ["command"]
                    }
                }
            })
        })
        .collect()
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig) -> Self {
        Self { config }
    }

    fn chat_url(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    }
}

#[derive(Default)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub struct SseChatAccumulator {
    thinking: String,
    content: String,
    tool_calls: BTreeMap<usize, ToolCallAccumulator>,
}

impl SseChatAccumulator {
    pub fn process_line(
        &mut self,
        line: &str,
        delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<Flow, String> {
        let Some(payload) = data_payload(line) else {
            return Ok(Flow::More);
        };
        if payload == "[DONE]" {
            return Ok(Flow::Done);
        }
        let val: Value = serde_json::from_str(payload).map_err(|e| format!("Failed to parse SSE JSON: {}", e))?;
        if let Some(err) = val.get("error") {
            let msg = err.get("message").and_then(Value::as_str).or_else(|| err.as_str());
            return Err(format!("OpenAI error: {}", msg.unwrap_or("unknown error")));
        }
        self.process_chunk(&val, delta_tx);
        Ok(Flow::More)
    }

    fn process_chunk(&mut self, val: &Value, delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>) {
        let Some(delta) = val.pointer("/choices/0/delta") else {
            return;
        };
        let reasoning = delta
            .get("reasoning")
            .or_else(|| delta.get("reasoning_content"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        if let Some(th) = reasoning {
            self.thinking.push_str(th);
            if let Some(tx) = delta_tx {
                let _ = tx.send(AgentEvent::ThoughtDelta(th.to_string()));
            }
        }
        if let Some(c) = delta.get("content").and_then(Value::as_str).filter(|s| !s.is_empty()) {
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
            if let Some(id) = tc.get("id").and_then(Value::as_str) {
                entry.id.push_str(id);
            }
            if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
                entry.name.push_str(name);
            }
            if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                entry.arguments.push_str(args);
            }
        }
    }

    pub fn finish(self) -> AssistantTurn {
        let (thinking, text) = finalize_thinking_and_content(self.thinking, self.content);
        let tool_calls = self
            .tool_calls
            .into_iter()
            .filter(|(_, tc)| !tc.name.is_empty())
            .map(|(idx, tc)| ToolCall {
                id: if tc.id.is_empty() { format!("call_{idx}") } else { tc.id },
                input: parse_tool_arg_command(&tc.arguments),
                name: tc.name,
            })
            .collect();
        AssistantTurn {
            thinking: thinking.map(|text| Thinking { text, signature: None }),
            text,
            tool_calls,
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

fn non_blank(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

fn finalize_thinking_and_content(thinking: String, content: String) -> (Option<String>, Option<String>) {
    if let Some(th) = non_blank(thinking) {
        return (Some(th), non_blank(content));
    }
    if let Some(start) = content.find("<think>")
        && let Some(end) = content.find("</think>")
    {
        let thought = content[start + 7..end].trim().to_string();
        let remainder = content[end + 8..].trim().to_string();
        return (Some(thought), non_blank(remainder));
    }
    (None, non_blank(content))
}

impl LlmProvider for OpenAiProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<AssistantTurn, String> {
        let payload = json!({
            "model": self.config.model,
            "messages": params.history.iter().map(format_message).collect::<Vec<_>>(),
            "tools": format_tools(params.tools),
            "stream": true,
        });
        let res = post_request(HttpRequest {
            url: &self.chat_url(),
            body: payload.to_string(),
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
        let mut acc = SseChatAccumulator::default();
        for_each_line(res, |line| acc.process_line(line, params.delta_tx)).await?;
        Ok(acc.finish())
    }
}

#[cfg(test)]
mod tests;
