//! Messages API request body: system blocks, tool names, and history conversion.

use serde_json::{Value, json};

use super::AuthMode;
use crate::agent::tool::ToolDefinition;
use crate::agent::{AgentMessage, AssistantTurn, ToolCall};
use crate::provider::ChatRequestParams;

pub const MAX_TOKENS: u64 = 8192;
const IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";
const SYSTEM_PROMPT: &str = "You are piro, a terminal coding agent. Run shell commands with the Bash tool when they \
help answer the user. Keep replies concise.";
const TOOL_NAMES: [(&str, &str); 1] = [("bash", "Bash")];

pub fn normalize_model_alias(model: &str) -> &str {
    match model {
        "default" | "sonnet" | "claude-sonnet-4-6" => "claude-sonnet-4-6",
        "claude-sonnet-4-5" => "claude-sonnet-4-5-20250514",
        "opus" | "claude-opus-4-6" => "claude-opus-4-6",
        "haiku" | "claude-haiku-4-5" => "claude-haiku-4-5",
        "sonnet-5" | "claude-sonnet-5" => "claude-sonnet-5",
        "opus-5" | "claude-opus-5" => "claude-opus-5",
        "opus-5-5" | "opus-5.5" | "claude-opus-5.5" | "claude-opus-5-5" => "claude-opus-5-5",
        "sonnet-5-5" | "sonnet-5.5" | "claude-sonnet-5.5" | "claude-sonnet-5-5" => "claude-sonnet-5-5",
        "haiku-5" | "claude-haiku-5" | "haiku-5-5" | "haiku-5.5" | "claude-haiku-5.5" | "claude-haiku-5-5" => {
            "claude-haiku-5-5"
        }
        "fable" | "claude-fable" | "claude-fable-5" | "claude-fable-5.1" | "claude-fable-5-1" => "claude-fable-5-1",
        "mythos" | "claude-mythos" | "claude-mythos-5" | "claude-mythos-5.1" | "claude-mythos-5-1" => {
            "claude-mythos-5-1"
        }
        other => other,
    }
}

pub fn to_claude_tool_name(name: &str) -> &str {
    TOOL_NAMES
        .iter()
        .find(|(piro, _)| *piro == name)
        .map(|&(_, claude)| claude)
        .unwrap_or(name)
}

pub fn from_claude_tool_name(name: &str) -> &str {
    TOOL_NAMES
        .iter()
        .find(|(_, claude)| claude.eq_ignore_ascii_case(name))
        .map(|&(piro, _)| piro)
        .unwrap_or(name)
}

pub fn build_body(model: &str, mode: AuthMode, params: &ChatRequestParams<'_>) -> Value {
    let mut tools: Vec<Value> = params.tools.iter().map(tool_json).collect();
    if let Some(last) = tools.last_mut() {
        last["cache_control"] = ephemeral();
    }
    let mut body = json!({
        "model": normalize_model_alias(model),
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "system": system_blocks(mode),
        "messages": convert_messages(params.history),
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    body
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

/// Subscription logins must open with Claude Code's identity block; the piro prompt follows and carries the cache marker.
fn system_blocks(mode: AuthMode) -> Value {
    let mut blocks = Vec::new();
    if mode == AuthMode::OAuth {
        blocks.push(json!({ "type": "text", "text": IDENTITY }));
    }
    blocks.push(json!({ "type": "text", "text": SYSTEM_PROMPT, "cache_control": ephemeral() }));
    Value::Array(blocks)
}

fn tool_json(tool: &ToolDefinition) -> Value {
    json!({
        "name": to_claude_tool_name(tool.name),
        "description": tool.description,
        "input_schema": {
            "type": "object",
            "properties": { "command": { "type": "string", "description": "Shell command to run" } },
            "required": ["command"],
        },
    })
}

pub fn convert_messages(history: &[AgentMessage]) -> Vec<Value> {
    let mut turns: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for msg in history {
        match msg {
            AgentMessage::User(text) => push_turn(&mut turns, "user", text_block(text)),
            AgentMessage::Assistant(turn) => push_turn(&mut turns, "assistant", assistant_blocks(turn)),
            AgentMessage::ToolResult {
                call_id,
                output,
                success,
            } => push_turn(&mut turns, "user", [tool_result(call_id, output, *success)]),
        }
    }
    turns
        .into_iter()
        .map(|(role, content)| json!({ "role": role, "content": content }))
        .collect()
}

/// Consecutive same-role messages merge: tool results followed by a prompt become one user turn.
fn push_turn(turns: &mut Vec<(&'static str, Vec<Value>)>, role: &'static str, blocks: impl IntoIterator<Item = Value>) {
    let blocks: Vec<Value> = blocks.into_iter().collect();
    if blocks.is_empty() {
        return;
    }
    match turns.last_mut() {
        Some((last, content)) if *last == role => content.extend(blocks),
        _ => turns.push((role, blocks)),
    }
}

fn text_block(text: &str) -> Option<Value> {
    (!text.trim().is_empty()).then(|| json!({ "type": "text", "text": text }))
}

/// Reasoning without a signature cannot be replayed, so it is dropped rather than sent as text.
fn assistant_blocks(turn: &AssistantTurn) -> Vec<Value> {
    let mut blocks = Vec::new();
    if let Some(thinking) = &turn.thinking
        && let Some(signature) = &thinking.signature
    {
        blocks.push(json!({ "type": "thinking", "thinking": thinking.text, "signature": signature }));
    }
    blocks.extend(turn.text.as_deref().and_then(text_block));
    blocks.extend(turn.tool_calls.iter().map(tool_use));
    blocks
}

fn tool_use(call: &ToolCall) -> Value {
    json!({
        "type": "tool_use",
        "id": call.id,
        "name": to_claude_tool_name(&call.name),
        "input": { "command": call.input },
    })
}

fn tool_result(call_id: &str, output: &str, success: bool) -> Value {
    let mut block = json!({ "type": "tool_result", "tool_use_id": call_id, "content": output });
    if !success {
        block["is_error"] = json!(true);
    }
    block
}
