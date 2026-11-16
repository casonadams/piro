use super::*;

#[test]
fn streams_reasoning_and_content_deltas() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut acc = SseChatAccumulator::default();
    let lines = [
        "data: {\"choices\":[{\"delta\":{\"reasoning\":\"Pondering\"}}]}",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello world\"}}]}",
    ];
    for line in lines {
        assert_eq!(acc.process_line(line, Some(&tx)).unwrap(), Flow::More);
    }
    assert_eq!(acc.process_line("data: [DONE]", Some(&tx)).unwrap(), Flow::Done);

    let turn = acc.finish();
    assert_eq!(turn.thinking.map(|t| t.text).as_deref(), Some("Pondering"));
    assert_eq!(turn.text.as_deref(), Some("Hello world"));
    assert!(matches!(rx.try_recv().unwrap(), AgentEvent::ThoughtDelta(t) if t == "Pondering"));
    assert!(matches!(rx.try_recv().unwrap(), AgentEvent::ResponseDelta(r) if r == "Hello world"));
}

#[test]
fn reads_deepseek_reasoning_content() {
    let mut acc = SseChatAccumulator::default();
    let line = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"Deep reasoning\"}}]}";
    acc.process_line(line, None).unwrap();
    assert_eq!(acc.finish().thinking.map(|t| t.text).as_deref(), Some("Deep reasoning"));
}

#[test]
fn assembles_parallel_tool_calls_with_their_ids() {
    let mut acc = SseChatAccumulator::default();
    let lines = [
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"bash","arguments":"{\"command\": "}}]}}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"bash","arguments":"{\"command\":\"pwd\"}"}}]}}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"echo 42\"}"}}]}}]}"#,
    ];
    for line in lines {
        acc.process_line(line, None).unwrap();
    }
    let calls = acc.finish().tool_calls;
    assert_eq!(calls.len(), 2);
    assert_eq!((calls[0].id.as_str(), calls[0].input.as_str()), ("call_a", "echo 42"));
    assert_eq!((calls[1].id.as_str(), calls[1].input.as_str()), ("call_b", "pwd"));
}

#[test]
fn splits_inline_think_tags() {
    let mut acc = SseChatAccumulator::default();
    let line = "data: {\"choices\":[{\"delta\":{\"content\":\"<think>Inline plan</think>Final answer\"}}]}";
    acc.process_line(line, None).unwrap();
    let turn = acc.finish();
    assert_eq!(turn.thinking.map(|t| t.text).as_deref(), Some("Inline plan"));
    assert_eq!(turn.text.as_deref(), Some("Final answer"));
}

#[test]
fn surfaces_structured_stream_errors() {
    let mut acc = SseChatAccumulator::default();
    let err = acc
        .process_line(r#"data: {"error":{"message":"model not found"}}"#, None)
        .unwrap_err();
    assert!(err.contains("model not found"));
}

#[test]
fn tool_results_reference_the_call_that_produced_them() {
    let call = ToolCall {
        id: "call_a".into(),
        name: "bash".into(),
        input: "ls".into(),
    };
    let assistant = format_message(&AgentMessage::Assistant(AssistantTurn {
        tool_calls: vec![call],
        ..Default::default()
    }));
    let result = format_message(&AgentMessage::ToolResult {
        call_id: "call_a".into(),
        output: "file".into(),
        success: true,
    });
    assert_eq!(assistant["tool_calls"][0]["id"], "call_a");
    assert_eq!(
        assistant["tool_calls"][0]["function"]["arguments"],
        r#"{"command":"ls"}"#
    );
    assert_eq!(result["tool_call_id"], "call_a");
}
