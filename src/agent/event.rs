use tokio::sync::oneshot;
use tokio::task::AbortHandle;

use crate::agent::message::{AgentMessage, ToolCall};
use crate::agent::tool::ToolExit;
use crate::provider::Usage;

#[derive(Debug)]
pub enum AgentEvent {
    ThoughtDelta(String),
    ResponseDelta(String),
    ToolStart(ToolCall),
    ToolOutput {
        call_id: String,
        line: String,
    },
    ToolDone {
        call_id: String,
        exit: ToolExit,
    },
    /// A message the engine appended to the conversation; the session mirrors these.
    Committed(AgentMessage),
    Compacted(Vec<AgentMessage>),
    Usage(Usage),
    Approval(ApprovalRequest),
    Notice(String),
    Finished,
    Error(String),
}

/// Dropping a request unanswered denies the call.
#[derive(Debug)]
pub struct ApprovalRequest {
    pub call: ToolCall,
    reply: oneshot::Sender<bool>,
}

impl ApprovalRequest {
    pub fn new(call: ToolCall) -> (Self, oneshot::Receiver<bool>) {
        let (reply, answer) = oneshot::channel();
        (Self { call, reply }, answer)
    }

    pub fn answer(self, allow: bool) {
        let _ = self.reply.send(allow);
    }
}

/// Cancelling aborts the engine task; the running tool's future is dropped with it, which stops the tool.
pub struct AgentHandle {
    pub(super) abort_handle: AbortHandle,
}

impl AgentHandle {
    pub fn cancel(&self) {
        self.abort_handle.abort();
    }
}
