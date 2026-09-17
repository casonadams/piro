use serde_json::{Value, json};

use crate::agent::backend::AgentMessage;
use crate::agent::http::{HttpRequest, post_json};
use crate::agent::provider::{LlmProvider, LlmResponse};
use crate::agent::react::ToolCallRequest;
use crate::agent::tool::ToolDefinition;

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
            .map(|msg| match msg {
                AgentMessage::User(u) => json!({ "role": "user", "content": u }),
                AgentMessage::Thought(t) => {
                    json!({ "role": "assistant", "content": format!("<think>{}</think>", t) })
                }
                AgentMessage::ToolCall { name, input } => {
                    json!({
                        "role": "assistant",
                        "tool_calls": [{
                            "id": format!("call_{}", name),
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": json!({ "command": input }).to_string()
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

fn parse_tool_call_item(call: &Value) -> Option<ToolCallRequest> {
    let func = call.get("function")?;
    let name = func.get("name")?.as_str()?.to_string();
    let args_val = func.get("arguments")?;
    let input = if let Some(s) = args_val.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(s) {
            parsed.get("command").and_then(Value::as_str).unwrap_or(s).to_string()
        } else {
            s.to_string()
        }
    } else {
        args_val
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some(ToolCallRequest { name, input })
}

fn extract_thinking_and_content(msg: &Value) -> (Option<String>, Option<String>) {
    let reasoning = msg.get("reasoning_content").and_then(Value::as_str).map(str::to_string);

    let raw_content = msg.get("content").and_then(Value::as_str);
    if let Some(content) = raw_content {
        if let Some(start) = content.find("<think>")
            && let Some(end) = content.find("</think>")
        {
            let thought = content[start + 7..end].trim().to_string();
            let remainder = content[end + 8..].trim().to_string();
            let rem_opt = if remainder.is_empty() { None } else { Some(remainder) };
            return (Some(thought), rem_opt);
        }
        (reasoning, Some(content.to_string()))
    } else {
        (reasoning, None)
    }
}

fn parse_openai_response(val: Value) -> Result<LlmResponse, String> {
    let choice = val
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| "OpenAI response missing 'choices'".to_string())?;

    let msg = choice
        .get("message")
        .ok_or_else(|| "Choice missing 'message'".to_string())?;

    let (thinking, content) = extract_thinking_and_content(msg);
    let tool_calls = msg
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_tool_call_item).collect())
        .unwrap_or_default();

    Ok(LlmResponse {
        thinking,
        tool_calls,
        content,
    })
}

impl LlmProvider for OpenAiProvider {
    async fn chat(&self, history: &[AgentMessage], tools: &[ToolDefinition]) -> Result<LlmResponse, String> {
        let url = format!("{}/v1/chat/completions", self.config.base_url.trim_end_matches('/'));
        let payload = json!({
            "model": self.config.model,
            "messages": self.format_messages(history),
            "tools": self.format_tools(tools)
        });
        let body_str = payload.to_string();
        let raw = post_json(HttpRequest {
            url: &url,
            body: &body_str,
            auth_token: self.config.api_key.as_deref(),
        })
        .await?;

        let body: Value = serde_json::from_str(&raw).map_err(|e| format!("Failed to parse OpenAI JSON: {}", e))?;

        parse_openai_response(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_openai_response_with_tools() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "Determining branch",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"git branch --show-current\"}"
                        }
                    }]
                }
            }]
        });
        let res = parse_openai_response(raw).unwrap();
        assert_eq!(res.thinking.as_deref(), Some("Determining branch"));
        assert_eq!(res.tool_calls.len(), 1);
        assert_eq!(res.tool_calls[0].name, "bash");
        assert_eq!(res.tool_calls[0].input, "git branch --show-current");
        assert_eq!(res.content, None);
    }

    #[test]
    fn test_parse_openai_response_think_tags() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "<think>Thinking through</think>The branch is main."
                }
            }]
        });
        let res = parse_openai_response(raw).unwrap();
        assert_eq!(res.thinking.as_deref(), Some("Thinking through"));
        assert_eq!(res.content.as_deref(), Some("The branch is main."));
        assert!(res.tool_calls.is_empty());
    }
}
