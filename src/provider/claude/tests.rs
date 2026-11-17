use serde_json::json;
use tokio::sync::mpsc;

use super::AuthMode;
use super::request::{build_body, convert_messages, normalize_model_alias};
use super::stream::StreamAccumulator;
use crate::agent::AgentEvent;
use crate::agent::tool::ToolDefinition;
use crate::agent::{AgentMessage, AssistantTurn, Thinking, ToolCall};
use crate::provider::sse::Flow;
use crate::provider::{ChatRequestParams, Reply, Usage};

fn bash() -> ToolDefinition {
    ToolDefinition {
        name: "bash".into(),
        description: "Run a shell command".into(),
        schema: json!({ "type": "object", "required": ["command"] }),
    }
}

fn params<'a>(history: &'a [AgentMessage], tools: &'a [ToolDefinition]) -> ChatRequestParams<'a> {
    ChatRequestParams {
        system: "be brief",
        history,
        tools,
        delta_tx: None,
    }
}

fn call(id: &str, command: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bash".into(),
        input: json!({ "command": command }),
    }
}

#[test]
fn oauth_body_opens_with_identity_and_marks_last_system_block_and_tool() {
    let history = [AgentMessage::User("hi".into())];
    let tools = [bash()];
    let body = build_body("sonnet", AuthMode::OAuth, &params(&history, &tools));
    assert_eq!(body["model"], "claude-sonnet-4-6");
    assert_eq!(body["max_tokens"], 32_000);
    assert_eq!(body["stream"], true);
    let system = body["system"].as_array().unwrap();
    assert_eq!(system.len(), 2);
    assert_eq!(
        system[0]["text"],
        "You are Claude Code, Anthropic's official CLI for Claude."
    );
    assert!(system[0].get("cache_control").is_none());
    assert_eq!(
        system[1],
        json!({ "type": "text", "text": "be brief", "cache_control": { "type": "ephemeral" } })
    );
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "Bash");
    assert_eq!(tools[0]["input_schema"]["required"], json!(["command"]));
    assert_eq!(tools[0]["cache_control"], json!({ "type": "ephemeral" }));
}

#[test]
fn api_key_body_has_no_identity_block_and_omits_empty_tools() {
    let history = [AgentMessage::User("hi".into())];
    let body = build_body("claude-opus-4-6", AuthMode::ApiKey, &params(&history, &[]));
    let system = body["system"].as_array().unwrap();
    assert_eq!(system.len(), 1);
    assert_ne!(
        system[0]["text"],
        "You are Claude Code, Anthropic's official CLI for Claude."
    );
    assert!(body.get("tools").is_none());
}

#[test]
fn normalizes_model_aliases() {
    assert_eq!(normalize_model_alias("haiku"), "claude-haiku-4-5");
    assert_eq!(normalize_model_alias("claude-haiku-5.5"), "claude-haiku-5-5");
    assert_eq!(
        normalize_model_alias("claude-3-7-sonnet-20250219"),
        "claude-3-7-sonnet-20250219"
    );
}

fn mixed_history() -> Vec<AgentMessage> {
    vec![
        AgentMessage::User("hi".into()),
        AgentMessage::Assistant(AssistantTurn {
            thinking: Some(Thinking {
                text: "plan".into(),
                signature: Some("SIG".into()),
            }),
            text: Some("Checking.".into()),
            tool_calls: vec![call("t1", "ls")],
        }),
        AgentMessage::ToolResult {
            call_id: "t1".into(),
            output: "boom".into(),
            success: false,
        },
        AgentMessage::User("next".into()),
        AgentMessage::Assistant(AssistantTurn {
            thinking: Some(Thinking {
                text: "unsigned".into(),
                signature: None,
            }),
            text: Some("   ".into()),
            tool_calls: vec![],
        }),
    ]
}

#[test]
fn converts_history_with_signed_thinking_errors_and_merged_turns() {
    let messages = convert_messages(&mixed_history());
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0],
        json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] })
    );
    assert_eq!(
        messages[1]["content"],
        json!([
            { "type": "thinking", "thinking": "plan", "signature": "SIG" },
            { "type": "text", "text": "Checking." },
            { "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } },
        ])
    );
    assert_eq!(
        messages[2],
        json!({
            "role": "user",
            "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "boom", "is_error": true },
                { "type": "text", "text": "next" },
            ]
        })
    );
}

#[test]
fn successful_tool_results_carry_no_error_flag() {
    let history = [AgentMessage::ToolResult {
        call_id: "a".into(),
        output: "ok".into(),
        success: true,
    }];
    let messages = convert_messages(&history);
    assert_eq!(
        messages[0]["content"][0],
        json!({ "type": "tool_result", "tool_use_id": "a", "content": "ok" })
    );
}

const RECORDED: &[&str] = &[
    "event: message_start",
    r#"data: {"type":"message_start","message":{"id":"m1","usage":{"input_tokens":10,"cache_read_input_tokens":90,"cache_creation_input_tokens":5,"output_tokens":1}}}"#,
    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"SIG"}}"#,
    r#"data: {"type":"content_block_stop","index":0}"#,
    r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
    r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Let me "}}"#,
    r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"check."}}"#,
    r#"data: {"type":"content_block_stop","index":1}"#,
    r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"Bash","input":{}}}"#,
    r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"comm"}}"#,
    r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"and\":\"ls\"}"}}"#,
    r#"data: {"type":"content_block_stop","index":2}"#,
    r#"data: {"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"t2","name":"Bash","input":{}}}"#,
    r#"data: {"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"pwd\"}"}}"#,
    r#"data: {"type":"content_block_stop","index":3}"#,
    r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":5}}"#,
    "event: message_stop",
    r#"data: {"type":"message_stop"}"#,
];

#[test]
fn accumulates_recorded_stream_into_turn_and_deltas() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut acc = StreamAccumulator::default();
    let mut flow = Flow::More;
    for line in RECORDED {
        flow = acc.process_line(line, Some(&tx)).unwrap();
    }
    assert_eq!(flow, Flow::Done);

    let (mut response, mut thought) = (String::new(), String::new());
    while let Ok(event) = rx.try_recv() {
        match event {
            AgentEvent::ResponseDelta(s) => response.push_str(&s),
            AgentEvent::ThoughtDelta(s) => thought.push_str(&s),
            _ => {}
        }
    }
    assert_eq!(response, "Let me check.");
    assert_eq!(thought, "hmm");

    assert_eq!(
        acc.finish(),
        Reply {
            turn: AssistantTurn {
                thinking: Some(Thinking {
                    text: "hmm".into(),
                    signature: Some("SIG".into()),
                }),
                text: Some("Let me check.".into()),
                tool_calls: vec![call("t1", "ls"), call("t2", "pwd")],
            },
            usage: Usage {
                input: 10,
                output: 5,
                cache_read: 90,
                cache_write: 5,
            },
            truncated: true,
        }
    );
}

#[test]
fn stream_error_event_becomes_an_error() {
    let mut acc = StreamAccumulator::default();
    let line = r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
    let err = acc.process_line(line, None).unwrap_err();
    assert!(err.contains("Overloaded"), "{err}");
}

#[test]
fn ignores_non_data_lines_and_keepalives() {
    let mut acc = StreamAccumulator::default();
    assert_eq!(acc.process_line("event: ping", None), Ok(Flow::More));
    assert_eq!(acc.process_line(r#"data: {"type":"ping"}"#, None), Ok(Flow::More));
    assert_eq!(acc.finish(), Reply::default());
}
