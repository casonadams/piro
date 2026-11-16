use crate::agent::{AgentEvent, AgentMessage, ToolCall};
use crate::app::channels::Incoming;
use crate::app::feed::{AgentSlot, CommandOutputBlock, FeedItem};
use crate::app::session::Session;
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
            AgentEvent::Notice(text) => {
                self.feed.push(FeedItem::AgentResponse(text));
            }
            AgentEvent::Finished | AgentEvent::Cancelled => self.finish_agent(),
            AgentEvent::Error(err) => {
                self.feed.push(FeedItem::AgentResponse(format!("Error: {err}")));
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

    /// Mirrors an engine message into the agent context; a committed assistant turn replaces its streamed cards.
    fn commit(&mut self, msg: AgentMessage) {
        if let AgentMessage::Assistant(turn) = &msg {
            if let Some(thinking) = &turn.thinking {
                self.feed.set_text(AgentSlot::Thought, thinking.text.clone());
            }
            if let Some(text) = &turn.text {
                self.feed.set_text(AgentSlot::Response, text.clone());
            }
        }
        self.agent_history.push(msg);
    }
}

fn tool_block(call: ToolCall) -> CommandOutputBlock {
    let mut block = CommandOutputBlock::new(call.input, Some(call.name));
    block.call_id = Some(call.id);
    block
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::agent::tool::ToolExit;
    use crate::agent::{AssistantTurn, Thinking};
    use crate::app::channels::channels;
    use crate::app::feed::CommandStatus;
    use crate::app::history::History;

    fn session() -> Session {
        let (channels, _inbox) = channels();
        Session::new("ollama/test".into(), History::default(), channels)
    }

    fn texts(s: &Session) -> Vec<String> {
        s.feed.entries().iter().map(|e| e.item.copy_text()).collect()
    }

    fn tool_call(id: &str, input: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "bash".into(),
            input: input.into(),
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
    fn finished_and_cancelled_end_the_agent_turn() {
        let mut s = session();
        s.apply_agent(AgentEvent::Finished);
        s.apply_agent(AgentEvent::Cancelled);
        assert_eq!(s.status(), crate::app::session::Status::Ready);
    }
}
