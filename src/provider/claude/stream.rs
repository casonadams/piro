//! Parses Messages API server-sent events into an `AssistantTurn`, streaming text and thinking as they arrive.

use std::collections::BTreeMap;

use serde_json::Value;
use tokio::sync::mpsc;

use super::request::from_claude_tool_name;
use crate::agent::AgentEvent;
use crate::agent::{AssistantTurn, Thinking, ToolCall};
use crate::provider::sse::{Flow, data_payload};
use crate::provider::{Reply, Usage, tool_input};

struct PendingTool {
    id: String,
    name: String,
    json: String,
}

#[derive(Default)]
pub struct StreamAccumulator {
    tool_uses: BTreeMap<u64, PendingTool>,
    thinking: String,
    signature: Option<String>,
    text: String,
    tool_calls: Vec<ToolCall>,
    usage: Usage,
    stop_reason: Option<String>,
}

impl StreamAccumulator {
    pub fn process_line(
        &mut self,
        line: &str,
        delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<Flow, String> {
        let Some(payload) = data_payload(line) else {
            return Ok(Flow::More);
        };
        let event: Value =
            serde_json::from_str(payload).map_err(|e| format!("Failed to parse Claude stream event: {e}"))?;
        match event["type"].as_str() {
            Some("message_start") => self.message_start(&event["message"]["usage"]),
            Some("message_delta") => self.message_delta(&event),
            Some("content_block_start") => self.block_start(&event),
            Some("content_block_delta") => self.block_delta(&event, delta_tx),
            Some("content_block_stop") => self.block_stop(&event),
            Some("message_stop") => return Ok(Flow::Done),
            Some("error") => {
                let message = event["error"]["message"].as_str().unwrap_or("unknown error");
                return Err(format!("Claude stream error: {message}"));
            }
            _ => {}
        }
        Ok(Flow::More)
    }

    fn message_start(&mut self, usage: &Value) {
        let count = |key: &str| usage[key].as_u64().unwrap_or_default();
        self.usage = Usage {
            input: count("input_tokens"),
            output: count("output_tokens"),
            cache_read: count("cache_read_input_tokens"),
            cache_write: count("cache_creation_input_tokens"),
        };
    }

    /// `message_delta` usage counts output tokens cumulatively.
    fn message_delta(&mut self, event: &Value) {
        if let Some(output) = event["usage"]["output_tokens"].as_u64() {
            self.usage.output = output;
        }
        if let Some(reason) = event["delta"]["stop_reason"].as_str() {
            self.stop_reason = Some(reason.to_owned());
        }
    }

    fn block_start(&mut self, event: &Value) {
        let (Some(index), block) = (event["index"].as_u64(), &event["content_block"]) else {
            return;
        };
        if block["type"].as_str() == Some("tool_use") {
            let pending = PendingTool {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: from_claude_tool_name(block["name"].as_str().unwrap_or_default()).to_string(),
                json: String::new(),
            };
            self.tool_uses.insert(index, pending);
        }
    }

    fn block_delta(&mut self, event: &Value, delta_tx: Option<&mpsc::UnboundedSender<AgentEvent>>) {
        let delta = &event["delta"];
        match delta["type"].as_str() {
            Some("text_delta") => {
                let text = delta["text"].as_str().unwrap_or_default();
                self.text.push_str(text);
                send(delta_tx, AgentEvent::ResponseDelta(text.to_string()));
            }
            Some("thinking_delta") => {
                let text = delta["thinking"].as_str().unwrap_or_default();
                self.thinking.push_str(text);
                send(delta_tx, AgentEvent::ThoughtDelta(text.to_string()));
            }
            Some("signature_delta") => {
                let signature = delta["signature"].as_str().unwrap_or_default();
                self.signature.get_or_insert_with(String::new).push_str(signature);
            }
            Some("input_json_delta") => {
                let partial = delta["partial_json"].as_str().unwrap_or_default();
                if let Some(tool) = event["index"].as_u64().and_then(|i| self.tool_uses.get_mut(&i)) {
                    tool.json.push_str(partial);
                }
            }
            _ => {}
        }
    }

    fn block_stop(&mut self, event: &Value) {
        let Some(tool) = event["index"].as_u64().and_then(|i| self.tool_uses.remove(&i)) else {
            return;
        };
        self.tool_calls.push(ToolCall {
            id: tool.id,
            name: tool.name,
            input: tool_input(&tool.json),
        });
    }

    pub fn finish(self) -> Reply {
        let thinking = if self.thinking.trim().is_empty() && self.signature.is_none() {
            None
        } else {
            Some(Thinking {
                text: self.thinking,
                signature: self.signature,
            })
        };
        Reply {
            turn: AssistantTurn {
                thinking,
                text: (!self.text.trim().is_empty()).then_some(self.text),
                tool_calls: self.tool_calls,
            },
            usage: self.usage,
            truncated: self.stop_reason.as_deref() == Some("max_tokens"),
        }
    }
}

fn send(tx: Option<&mpsc::UnboundedSender<AgentEvent>>, event: AgentEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}
