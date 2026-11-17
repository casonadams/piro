use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

impl ToolCall {
    pub fn summary(&self) -> String {
        ["command", "path", "description"]
            .iter()
            .find_map(|key| self.input.get(key).and_then(Value::as_str))
            .map_or_else(|| self.input.to_string(), str::to_owned)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thinking {
    pub text: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantTurn {
    pub thinking: Option<Thinking>,
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentMessage {
    User(String),
    Assistant(AssistantTurn),
    ToolResult {
        call_id: String,
        output: String,
        success: bool,
    },
}

pub const CANCELLED_TOOL_OUTPUT: &str = "Tool call cancelled by the user.";

/// Providers reject a tool call without a result, so a cancelled turn gets one per open call.
pub fn close_dangling_tool_calls(history: &mut Vec<AgentMessage>) {
    let mut open: Vec<String> = Vec::new();
    let mut closed = Vec::with_capacity(history.len());
    for msg in history.drain(..) {
        match &msg {
            AgentMessage::ToolResult { call_id, .. } => open.retain(|id| id != call_id),
            AgentMessage::Assistant(turn) => {
                push_cancelled(&mut closed, &mut open);
                open.extend(turn.tool_calls.iter().map(|c| c.id.clone()));
            }
            AgentMessage::User(_) => push_cancelled(&mut closed, &mut open),
        }
        closed.push(msg);
    }
    push_cancelled(&mut closed, &mut open);
    *history = closed;
}

fn push_cancelled(history: &mut Vec<AgentMessage>, open: &mut Vec<String>) {
    history.extend(open.drain(..).map(|call_id| AgentMessage::ToolResult {
        call_id,
        output: CANCELLED_TOOL_OUTPUT.into(),
        success: false,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "bash".into(),
            input: "ls".into(),
        }
    }

    fn result(id: &str, output: &str) -> AgentMessage {
        AgentMessage::ToolResult {
            call_id: id.into(),
            output: output.into(),
            success: output != CANCELLED_TOOL_OUTPUT,
        }
    }

    #[test]
    fn closes_only_unanswered_calls_before_the_next_message() {
        let mut history = vec![
            AgentMessage::User("go".into()),
            AgentMessage::Assistant(AssistantTurn {
                tool_calls: vec![call("a"), call("b")],
                ..Default::default()
            }),
            result("a", "ok"),
            AgentMessage::User("next".into()),
        ];
        close_dangling_tool_calls(&mut history);
        assert_eq!(history[2], result("a", "ok"));
        assert_eq!(history[3], result("b", CANCELLED_TOOL_OUTPUT));
        assert_eq!(history[4], AgentMessage::User("next".into()));
        assert_eq!(history.len(), 5);
    }

    #[test]
    fn closes_trailing_calls_and_leaves_complete_history_untouched() {
        let complete = vec![
            AgentMessage::Assistant(AssistantTurn {
                tool_calls: vec![call("a")],
                ..Default::default()
            }),
            result("a", "ok"),
        ];
        let mut history = complete.clone();
        close_dangling_tool_calls(&mut history);
        assert_eq!(history, complete);

        history.push(AgentMessage::Assistant(AssistantTurn {
            tool_calls: vec![call("c")],
            ..Default::default()
        }));
        close_dangling_tool_calls(&mut history);
        assert_eq!(history.last(), Some(&result("c", CANCELLED_TOOL_OUTPUT)));
    }
}
