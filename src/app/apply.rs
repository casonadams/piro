use std::time::Duration;

use crate::agent::tool::ToolExit;
use crate::agent::{AgentEvent, AgentMessage, ToolCall};
use crate::app::channels::Incoming;
use crate::app::feed::{AgentSlot, CommandOutputBlock, FeedItem};
use crate::app::session::Session;
use crate::provider::Usage;
use crate::sys::RunnerMsg;

impl Session {
    pub fn apply(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Runner(msg) => self.apply_runner(msg),
            Incoming::Agent(event) => self.apply_agent(event),
        }
    }

    pub fn apply_runner(&mut self, msg: RunnerMsg) {
        match msg {
            RunnerMsg::Line(line) => self.push_command_line(line),
            RunnerMsg::Done(code, duration) => self.command_finished(code, duration),
        }
    }

    pub fn apply_agent(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::ThoughtDelta(delta) => self.feed.append_delta(AgentSlot::Thought, &delta),
            AgentEvent::ResponseDelta(delta) => self.feed.append_delta(AgentSlot::Response, &delta),
            AgentEvent::ToolStart(call) => {
                self.feed.push(FeedItem::CommandOutput(tool_block(call)));
            }
            AgentEvent::ToolOutput { call_id, line } => self.with_tool(&call_id, |b| b.push_line(line)),
            AgentEvent::ToolDone { call_id, exit } => self.with_tool(&call_id, |b| b.complete(exit)),
            AgentEvent::Committed(msg) => self.commit(msg),
            AgentEvent::Compacted(history) => self.compacted(history),
            AgentEvent::Usage(usage) => self.record_usage(usage),
            AgentEvent::Approval(request) => self.request_approval(request),
            AgentEvent::Notice(text) => self.notice(text),
            AgentEvent::Finished => self.finish_agent(),
            AgentEvent::Error(err) => {
                self.notice(format!("Error: {err}"));
                self.finish_agent();
            }
        }
    }

    fn with_tool(&mut self, call_id: &str, f: impl FnOnce(&mut CommandOutputBlock)) {
        if let Some(block) = self
            .feed
            .tool_block_id(call_id)
            .and_then(|id| self.feed.command_mut(id))
        {
            f(block);
        }
    }

    /// Mirrors an engine message into the agent context and the saved conversation; a committed assistant
    /// turn replaces its streamed cards, and a tool result fills its card when the tool streamed nothing.
    fn commit(&mut self, msg: AgentMessage) {
        match &msg {
            AgentMessage::Assistant(turn) => {
                if let Some(thinking) = &turn.thinking {
                    self.feed.set_text(AgentSlot::Thought, thinking.text.clone());
                }
                if let Some(text) = &turn.text {
                    self.feed.set_text(AgentSlot::Response, text.clone());
                }
            }
            AgentMessage::ToolResult { call_id, output, .. } => self.with_tool(call_id, |b| fill_empty(b, output)),
            AgentMessage::User(_) => {}
        }
        self.transcript.append(&msg);
        self.agent_history.push(msg);
    }

    fn compacted(&mut self, history: Vec<AgentMessage>) {
        self.transcript.rewrite(&history);
        self.agent_history = history;
        self.context_tokens = 0;
        self.notice("Conversation compacted.".into());
    }

    fn record_usage(&mut self, usage: Usage) {
        self.tokens.input += usage.input;
        self.tokens.output += usage.output;
        self.tokens.cache_read += usage.cache_read;
        self.tokens.cache_write += usage.cache_write;
        self.context_tokens = usage.context();
    }

    pub(crate) fn replay(&mut self, messages: Vec<AgentMessage>) {
        for msg in messages {
            match &msg {
                AgentMessage::User(text) => {
                    self.feed.push(FeedItem::UserInput(text.clone()));
                }
                AgentMessage::Assistant(turn) => {
                    if let Some(text) = &turn.text {
                        self.feed.push(FeedItem::AgentResponse(text.clone()));
                    }
                    for call in &turn.tool_calls {
                        self.feed.push(FeedItem::CommandOutput(tool_block(call.clone())));
                    }
                }
                AgentMessage::ToolResult {
                    call_id,
                    output,
                    success,
                } => self.with_tool(call_id, |block| replay_result(block, output, *success)),
            }
            self.agent_history.push(msg);
        }
    }
}

fn replay_result(block: &mut CommandOutputBlock, output: &str, success: bool) {
    fill_empty(block, output);
    block.complete(ToolExit {
        code: Some(if success { 0 } else { 1 }),
        duration: Duration::ZERO,
    });
}

fn fill_empty(block: &mut CommandOutputBlock, output: &str) {
    if block.lines.is_empty() {
        output.lines().for_each(|line| block.push_line(line.to_owned()));
    }
}

fn tool_block(call: ToolCall) -> CommandOutputBlock {
    let mut block = CommandOutputBlock::new(call.summary(), Some(call.name));
    block.call_id = Some(call.id);
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{ApprovalRequest, AssistantTurn, Thinking};
    use crate::app::channels::channels;
    use crate::app::feed::CommandStatus;
    use crate::app::history::History;
    use crate::app::session::{Approval, Status};
    use crate::launch::SessionConfig;

    fn session() -> Session {
        let (channels, _inbox) = channels();
        let config = SessionConfig {
            model_spec: "ollama/test".into(),
            ..Default::default()
        };
        Session::new(config, History::default(), channels)
    }

    fn texts(s: &Session) -> Vec<String> {
        s.feed.entries().iter().map(|e| e.item.copy_text()).collect()
    }

    fn tool_call(id: &str, input: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": input }),
        }
    }

    fn exit(code: i32) -> ToolExit {
        ToolExit {
            code: Some(code),
            duration: Duration::from_millis(5),
        }
    }

    #[test]
    fn deltas_then_commit_do_not_duplicate_cards() {
        let mut s = session();
        s.feed.push(FeedItem::UserInput("question".into()));
        s.apply_agent(AgentEvent::ThoughtDelta("Think ".into()));
        s.apply_agent(AgentEvent::ThoughtDelta("hard.".into()));
        s.apply_agent(AgentEvent::ResponseDelta("Answer ".into()));
        s.apply_agent(AgentEvent::ResponseDelta("here.".into()));
        assert_eq!(s.feed.entries().len(), 3);

        s.apply_agent(AgentEvent::Committed(AgentMessage::Assistant(AssistantTurn {
            thinking: Some(Thinking {
                text: "Think hard.".into(),
                signature: None,
            }),
            text: Some("Answer here.".into()),
            tool_calls: Vec::new(),
        })));
        assert_eq!(texts(&s), ["question", "Think hard.", "Answer here."]);
        assert_eq!(s.agent_history.len(), 1);
    }

    #[test]
    fn tool_lines_land_in_the_block_with_their_call_id() {
        let mut s = session();
        s.apply_agent(AgentEvent::ToolStart(tool_call("a", "ls")));
        s.apply_agent(AgentEvent::ToolStart(tool_call("b", "pwd")));
        s.apply_agent(AgentEvent::ToolOutput {
            call_id: "a".into(),
            line: "file".into(),
        });
        s.apply_agent(AgentEvent::ToolOutput {
            call_id: "b".into(),
            line: "/tmp".into(),
        });
        s.apply_agent(AgentEvent::ToolDone {
            call_id: "a".into(),
            exit: exit(0),
        });
        let a = s.feed.tool_block_id("a").and_then(|id| s.feed.get(id));
        let FeedItem::CommandOutput(a) = a.expect("block a") else {
            panic!("expected a command block");
        };
        assert_eq!(a.lines, ["file"]);
        assert_eq!(a.status, CommandStatus::Success(0));
        let b = s.feed.tool_block_id("b").and_then(|id| s.feed.get(id));
        let FeedItem::CommandOutput(b) = b.expect("block b") else {
            panic!("expected a command block");
        };
        assert_eq!(b.lines, ["/tmp"]);
        assert_eq!(b.status, CommandStatus::Running);
    }

    #[test]
    fn notice_and_error_render_as_agent_responses() {
        let mut s = session();
        s.apply_agent(AgentEvent::Notice("note".into()));
        s.apply_agent(AgentEvent::Error("boom".into()));
        assert_eq!(texts(&s), ["note", "Error: boom"]);
        assert!(
            s.feed
                .entries()
                .iter()
                .all(|e| matches!(e.item, FeedItem::AgentResponse(_)))
        );
    }

    #[test]
    fn tool_failure_marks_block_failed() {
        let mut s = session();
        s.apply_agent(AgentEvent::ToolStart(tool_call("x", "false")));
        s.apply_agent(AgentEvent::ToolDone {
            call_id: "x".into(),
            exit: exit(1),
        });
        let id = s.feed.tool_block_id("x").expect("block");
        let FeedItem::CommandOutput(block) = s.feed.get(id).expect("entry") else {
            panic!("expected a command block");
        };
        assert_eq!(block.status, CommandStatus::Failed(1));
    }

    #[test]
    fn finished_ends_the_agent_turn() {
        let mut s = session();
        s.apply_agent(AgentEvent::Finished);
        assert_eq!(s.status(), Status::Ready);
    }

    #[test]
    fn always_allow_answers_later_calls_of_the_same_tool() {
        let mut s = session();
        let (first, mut first_rx) = ApprovalRequest::new(tool_call("a", "ls"));
        s.apply_agent(AgentEvent::Approval(first));
        assert_eq!(s.status(), Status::Approval);
        assert_eq!(s.approval_prompt().as_deref(), Some("bash: ls"));
        s.answer_approval(Approval::AlwaysAllow);
        assert_eq!(first_rx.try_recv(), Ok(true));

        let (second, mut second_rx) = ApprovalRequest::new(tool_call("b", "rm x"));
        s.apply_agent(AgentEvent::Approval(second));
        assert_eq!(second_rx.try_recv(), Ok(true));
        assert_eq!(s.status(), Status::Ready);

        let write = ToolCall {
            id: "c".into(),
            name: "write".into(),
            input: serde_json::json!({ "path": "f", "content": "x" }),
        };
        let (third, mut third_rx) = ApprovalRequest::new(write);
        s.apply_agent(AgentEvent::Approval(third));
        assert!(s.approval_prompt().is_some_and(|p| p.starts_with("write: f\n{")));
        s.answer_approval(Approval::Deny);
        assert_eq!(third_rx.try_recv(), Ok(false));
    }

    #[test]
    fn replay_rebuilds_cards_and_context_from_a_saved_conversation() {
        let mut s = session();
        let messages = vec![
            AgentMessage::User("list".into()),
            AgentMessage::Assistant(AssistantTurn {
                tool_calls: vec![tool_call("a", "ls")],
                ..Default::default()
            }),
            AgentMessage::ToolResult {
                call_id: "a".into(),
                output: "one\ntwo".into(),
                success: false,
            },
            AgentMessage::Assistant(AssistantTurn {
                text: Some("Two files.".into()),
                ..Default::default()
            }),
        ];
        s.replay(messages.clone());
        assert_eq!(texts(&s), ["list", "one\ntwo", "Two files."]);
        let id = s.feed.tool_block_id("a").expect("block");
        assert!(matches!(s.feed.get(id), Some(FeedItem::CommandOutput(b)) if b.status == CommandStatus::Failed(1)));
        assert_eq!(s.agent_history, messages);
    }
}
