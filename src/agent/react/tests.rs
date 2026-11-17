use std::time::Duration;

use serde_json::json;

use super::*;
use crate::agent::tool::Workspace;
use crate::provider::{MockProvider, ProviderError};

fn bash(id: &str, cmd: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bash".into(),
        input: json!({ "command": cmd }),
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

fn options(job: Job, agent_tx: mpsc::UnboundedSender<AgentEvent>) -> EngineSpawnOptions {
    EngineSpawnOptions {
        job,
        history: Vec::new(),
        agent_tx,
        auto_approve: true,
        context_tokens: 0,
    }
}

fn spawn(provider: MockProvider, opts: EngineSpawnOptions) -> AgentHandle {
    let registry = ToolRegistry::with_defaults(Workspace::new(std::env::temp_dir()), Vec::new());
    ReActEngine::new(provider, registry).spawn_with_options(opts)
}

async fn run(provider: MockProvider, prompt: &str) -> Vec<AgentEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    let _handle = spawn(provider, options(Job::Prompt(prompt.into()), tx));
    collect_events(rx).await
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

fn notices(events: &[AgentEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Notice(n) => Some(n.as_str()),
            _ => None,
        })
        .collect()
}

fn reply(turn: AssistantTurn) -> Result<Reply, ProviderError> {
    Ok(Reply {
        turn,
        ..Default::default()
    })
}

#[tokio::test]
async fn runs_every_tool_call_of_a_turn_before_asking_the_model_again() {
    let provider = MockProvider::from_turns(vec![
        tool_turn(vec![bash("c1", "echo one"), bash("c2", "echo two; exit 4")]),
        text_turn("both done"),
    ]);
    let calls = provider.call_counter();
    let events = run(provider, "go").await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let history = committed(&events);
    assert_eq!(history.len(), 5);
    assert_eq!(history[0], AgentMessage::User("go".into()));
    assert!(
        matches!(&history[2], AgentMessage::ToolResult { call_id, output, success: true } if call_id == "c1" && output == "one")
    );
    assert!(
        matches!(&history[3], AgentMessage::ToolResult { call_id, output, success: false } if call_id == "c2" && output == "two\n[exit code 4]")
    );
    assert_eq!(history[4], AgentMessage::Assistant(text_turn("both done")));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolOutput { call_id, line } if call_id == "c2" && line == "two"))
    );
}

#[tokio::test]
async fn denied_tool_is_answered_with_a_failed_result_and_never_runs() {
    let provider = MockProvider::from_turns(vec![tool_turn(vec![bash("c1", "echo blocked")])]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut opts = options(Job::Prompt("t".into()), tx);
    opts.auto_approve = false;
    let _handle = spawn(provider, opts);

    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        match event {
            AgentEvent::Approval(request) => {
                assert_eq!(request.call, bash("c1", "echo blocked"));
                request.answer(false);
            }
            AgentEvent::Finished => break,
            other => events.push(other),
        }
    }
    assert!(
        committed(&events)
            .iter()
            .any(|m| matches!(m, AgentMessage::ToolResult { call_id, success: false, output } if call_id == "c1" && output == DENIED))
    );
    assert!(!events.iter().any(|e| matches!(e, AgentEvent::ToolStart(_))));
}

#[tokio::test]
async fn read_only_tools_run_without_asking() {
    let read = ToolCall {
        id: "r".into(),
        name: "read".into(),
        input: json!({ "path": "/nonexistent-piro-file" }),
    };
    let provider = MockProvider::from_turns(vec![tool_turn(vec![read]), text_turn("ok")]);
    let (tx, rx) = mpsc::unbounded_channel();
    let mut opts = options(Job::Prompt("t".into()), tx);
    opts.auto_approve = false;
    let _handle = spawn(provider, opts);
    let events = collect_events(rx).await;
    assert!(!events.iter().any(|e| matches!(e, AgentEvent::Approval(_))));
    assert!(committed(&events).iter().any(
        |m| matches!(m, AgentMessage::ToolResult { success: false, output, .. } if output.starts_with("Cannot read"))
    ));
}

#[tokio::test]
async fn repeated_identical_calls_stop_the_loop_without_committing_the_turn() {
    let turns = (0..5)
        .map(|i| tool_turn(vec![bash(&format!("c{i}"), "true")]))
        .collect();
    let events = run(MockProvider::from_turns(turns), "t").await;
    assert!(notices(&events).iter().any(|n| n.contains("repetitive")));
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
    let events = run(provider, "what is this").await;
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
    let events = run(provider, "t").await;
    assert!(notices(&events).contains(&SILENT_NOTICE));
    assert!(!committed(&events).contains(&AgentMessage::Assistant(text_turn("late"))));
}

#[tokio::test]
async fn retryable_errors_are_retried_and_others_end_the_turn() {
    let overloaded = ProviderError {
        message: "Overloaded.".into(),
        retryable: true,
        retry_after: Some(Duration::ZERO),
    };
    let provider = MockProvider::from_replies(vec![Err(overloaded), reply(text_turn("recovered"))]);
    let events = run(provider, "t").await;
    assert_eq!(notices(&events), ["Overloaded. Retrying in 0s (1/4)."]);
    assert_eq!(
        committed(&events).last(),
        Some(&AgentMessage::Assistant(text_turn("recovered")))
    );

    let provider = MockProvider::from_replies(vec![Err("bad request".to_string().into())]);
    let events = run(provider, "t").await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Error(m) if m == "bad request"))
    );
}

#[tokio::test]
async fn truncated_reply_drops_its_tool_calls_and_asks_to_continue() {
    let mut cut = text_turn("Writing the file");
    cut.tool_calls.push(bash("c1", "echo partial"));
    let provider = MockProvider::from_replies(vec![
        Ok(Reply {
            turn: cut,
            truncated: true,
            ..Default::default()
        }),
        reply(text_turn("done")),
    ]);
    let seen = provider.seen_histories();
    let events = run(provider, "t").await;
    assert!(!events.iter().any(|e| matches!(e, AgentEvent::ToolStart(_))));
    let second = seen.lock().unwrap()[1].clone();
    assert_eq!(second[1], AgentMessage::Assistant(text_turn("Writing the file")));
    assert_eq!(second[2], AgentMessage::User(TRUNCATED_PROMPT.into()));
}

#[tokio::test]
async fn full_context_is_compacted_before_the_new_prompt() {
    let provider = MockProvider::from_turns(vec![text_turn("the summary"), text_turn("answer")]);
    let seen = provider.seen_histories();
    let (tx, rx) = mpsc::unbounded_channel();
    let mut opts = options(Job::Prompt("next".into()), tx);
    opts.history = vec![
        AgentMessage::User("old".into()),
        AgentMessage::Assistant(text_turn("old answer")),
    ];
    opts.context_tokens = 10_000_000;
    let _handle = spawn(provider, opts);
    let events = collect_events(rx).await;

    let compacted = events.iter().find_map(|e| match e {
        AgentEvent::Compacted(history) => Some(history.clone()),
        _ => None,
    });
    let compacted = compacted.expect("the conversation is compacted");
    assert!(matches!(&compacted[..], [AgentMessage::User(s)] if s.ends_with("the summary")));
    let answered = seen.lock().unwrap()[1].clone();
    assert_eq!(answered[1], AgentMessage::User("next".into()));
    assert_eq!(answered.len(), 2);
}

#[tokio::test]
async fn task_runs_a_subagent_and_returns_its_answer() {
    let task = ToolCall {
        id: "t1".into(),
        name: "task".into(),
        input: json!({ "description": "look", "prompt": "find it" }),
    };
    let provider = MockProvider::from_turns(vec![
        tool_turn(vec![task]),
        tool_turn(vec![bash("s1", "echo inside")]),
        text_turn("found it"),
        text_turn("parent done"),
    ]);
    let seen = provider.seen_histories();
    let events = run(provider, "go").await;
    assert!(
        committed(&events)
            .iter()
            .any(|m| matches!(m, AgentMessage::ToolResult { call_id, output, success: true } if call_id == "t1" && output == "found it"))
    );
    assert!(events.iter().any(
        |e| matches!(e, AgentEvent::ToolOutput { call_id, line } if call_id == "t1" && line == "bash: echo inside")
    ));
    assert_eq!(seen.lock().unwrap()[1], [AgentMessage::User("find it".into())]);
}

#[tokio::test]
async fn prior_history_with_cancelled_call_is_closed_before_the_new_prompt() {
    let provider = MockProvider::from_turns(vec![text_turn("Got it")]);
    let seen = provider.seen_histories();
    let (tx, rx) = mpsc::unbounded_channel();
    let mut opts = options(Job::Prompt("second".into()), tx);
    opts.history = vec![
        AgentMessage::User("first".into()),
        AgentMessage::Assistant(tool_turn(vec![bash("old", "sleep 9")])),
    ];
    let _handle = spawn(provider, opts);

    collect_events(rx).await;
    let sent = seen.lock().unwrap()[0].clone();
    assert!(matches!(&sent[2], AgentMessage::ToolResult { call_id, success: false, .. } if call_id == "old"));
    assert_eq!(sent[3], AgentMessage::User("second".into()));
}

#[tokio::test]
async fn cancel_kills_the_running_tool() {
    let marker = std::env::temp_dir().join(format!("piro-cancel-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let script = format!("sleep 2 && touch {}", marker.display());
    let provider = MockProvider::from_turns(vec![tool_turn(vec![bash("c1", &script)])]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = spawn(provider, options(Job::Prompt("t".into()), tx));
    while let Some(ev) = rx.recv().await {
        if matches!(ev, AgentEvent::ToolStart(_)) {
            break;
        }
    }
    handle.cancel();
    while rx.recv().await.is_some() {}
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!marker.exists(), "the killed command still ran to completion");
}
