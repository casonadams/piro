use std::time::Duration;

use super::*;
use crate::provider::MockProvider;

#[derive(Default)]
struct TrackingHooks {
    block_tools: bool,
}

impl ReActHooks for TrackingHooks {
    fn before_tool(&mut self, _call: &ToolCall) -> bool {
        !self.block_tools
    }
}

fn bash(id: &str, cmd: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bash".into(),
        input: cmd.into(),
    }
}

fn tool_turn(calls: Vec<ToolCall>) -> AssistantTurn {
    AssistantTurn {
        tool_calls: calls,
        ..Default::default()
    }
}

fn text_turn(text: &str) -> AssistantTurn {
    AssistantTurn {
        text: Some(text.into()),
        ..Default::default()
    }
}

async fn collect_events(mut rx: mpsc::UnboundedReceiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        let fin = matches!(ev, AgentEvent::Finished);
        events.push(ev);
        if fin {
            break;
        }
    }
    events
}

fn committed(events: &[AgentEvent]) -> Vec<AgentMessage> {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Committed(m) => Some(m.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn runs_every_tool_call_of_a_turn_before_asking_the_model_again() {
    let provider = MockProvider::from_turns(vec![
        tool_turn(vec![bash("c1", "echo one"), bash("c2", "echo two")]),
        text_turn("both done"),
    ]);
    let calls = provider.call_counter();
    let (tx, rx) = mpsc::unbounded_channel();
    let _handle = ReActEngine::new(provider, ToolRegistry::with_defaults()).spawn("go".into(), tx);

    let events = collect_events(rx).await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let history = committed(&events);
    assert_eq!(history.len(), 5);
    assert_eq!(history[0], AgentMessage::User("go".into()));
    assert!(
        matches!(&history[2], AgentMessage::ToolResult { call_id, output, success: true } if call_id == "c1" && output == "one")
    );
    assert!(
        matches!(&history[3], AgentMessage::ToolResult { call_id, output, success: true } if call_id == "c2" && output == "two")
    );
    assert_eq!(history[4], AgentMessage::Assistant(text_turn("both done")));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolOutput { call_id, line } if call_id == "c2" && line == "two"))
    );
}

#[tokio::test]
async fn blocked_tool_is_answered_with_a_failed_result() {
    let provider = MockProvider::from_turns(vec![tool_turn(vec![bash("c1", "echo blocked")])]);
    let hooks = TrackingHooks { block_tools: true };
    let (tx, rx) = mpsc::unbounded_channel();
    let _handle = ReActEngine::with_hooks(provider, ToolRegistry::with_defaults(), hooks).spawn("t".into(), tx);

    let events = collect_events(rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Error(err) if err.contains("blocked by policy")))
    );
    assert!(
        committed(&events)
            .iter()
            .any(|m| matches!(m, AgentMessage::ToolResult { call_id, success: false, .. } if call_id == "c1"))
    );
    assert!(!events.iter().any(|e| matches!(e, AgentEvent::ToolStart(_))));
}

#[tokio::test]
async fn repeated_identical_calls_stop_the_loop_without_committing_the_turn() {
    let turns = (0..5)
        .map(|i| tool_turn(vec![bash(&format!("c{i}"), "true")]))
        .collect();
    let (tx, rx) = mpsc::unbounded_channel();
    let _handle =
        ReActEngine::new(MockProvider::from_turns(turns), ToolRegistry::with_defaults()).spawn("t".into(), tx);

    let events = collect_events(rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Notice(n) if n.contains("repetitive")))
    );
    let turns_committed = committed(&events)
        .iter()
        .filter(|m| matches!(m, AgentMessage::Assistant(_)))
        .count();
    assert_eq!(turns_committed, 2);
}

fn thinking_turn() -> AssistantTurn {
    AssistantTurn {
        thinking: Some(crate::agent::Thinking {
            text: "I should look at the files".into(),
            signature: None,
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn reasoning_only_turn_asks_again_instead_of_stopping() {
    let provider = MockProvider::from_turns(vec![
        tool_turn(vec![bash("c1", "echo listed")]),
        thinking_turn(),
        tool_turn(vec![bash("c2", "echo read")]),
        text_turn("it is piro"),
    ]);
    let calls = provider.call_counter();
    let (tx, rx) = mpsc::unbounded_channel();
    let _handle = ReActEngine::new(provider, ToolRegistry::with_defaults()).spawn("what is this".into(), tx);

    let events = collect_events(rx).await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    let history = committed(&events);
    assert_eq!(history.last(), Some(&AgentMessage::Assistant(text_turn("it is piro"))));
    assert!(!history.contains(&AgentMessage::Assistant(thinking_turn())));
}

#[tokio::test]
async fn repeated_reasoning_only_turns_end_with_a_notice() {
    let provider = MockProvider::from_turns(vec![
        thinking_turn(),
        thinking_turn(),
        thinking_turn(),
        text_turn("late"),
    ]);
    let (tx, rx) = mpsc::unbounded_channel();
    let _handle = ReActEngine::new(provider, ToolRegistry::with_defaults()).spawn("t".into(), tx);

    let events = collect_events(rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Notice(n) if n == SILENT_NOTICE))
    );
    assert!(!committed(&events).contains(&AgentMessage::Assistant(text_turn("late"))));
}

#[tokio::test]
async fn prior_history_with_cancelled_call_is_closed_before_the_new_prompt() {
    let provider = MockProvider::from_turns(vec![text_turn("Got it")]);
    let seen = provider.seen_histories();
    let prior = vec![
        AgentMessage::User("first".into()),
        AgentMessage::Assistant(tool_turn(vec![bash("old", "sleep 9")])),
    ];
    let (tx, rx) = mpsc::unbounded_channel();
    let _handle = ReActEngine::new(provider, ToolRegistry::with_defaults()).spawn_with_options(EngineSpawnOptions {
        prompt: "second".into(),
        history: prior,
        agent_tx: tx,
    });

    collect_events(rx).await;
    let sent = seen.lock().unwrap()[0].clone();
    assert!(matches!(&sent[2], AgentMessage::ToolResult { call_id, success: false, .. } if call_id == "old"));
    assert_eq!(sent[3], AgentMessage::User("second".into()));
}

#[tokio::test]
async fn cancel_kills_the_running_tool() {
    let provider = MockProvider::from_turns(vec![tool_turn(vec![bash("c1", "sleep 10")])]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = ReActEngine::new(provider, ToolRegistry::with_defaults()).spawn("t".into(), tx);
    while let Some(ev) = rx.recv().await {
        if matches!(ev, AgentEvent::ToolStart(_)) {
            break;
        }
    }
    let started = std::time::Instant::now();
    handle.cancel();
    while rx.recv().await.is_some() {}
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn truncates_long_observations_to_head_and_tail() {
    let short = "line 1\nline 2";
    assert_eq!(truncate_observation_output(short), short);

    let long = (0..300).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
    let truncated = truncate_observation_output(&long);
    let lines: Vec<&str> = truncated.lines().collect();
    assert_eq!(lines.len(), 200);
    assert_eq!(lines[119], "line 119");
    assert_eq!(lines[120], "... [output truncated: 101 lines omitted] ...");
    assert_eq!(lines[121], "line 221");
    assert_eq!(lines[199], "line 299");
}

#[test]
fn repetition_guard_trips_on_third_identical_call() {
    let mut guard = RepetitionGuard::default();
    assert!(!guard.is_repetitive("bash", "ls"));
    assert!(!guard.is_repetitive("bash", "ls"));
    assert!(guard.is_repetitive("bash", "ls"));
    assert!(!guard.is_repetitive("bash", "pwd"));
}
