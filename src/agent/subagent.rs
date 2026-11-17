use std::pin::Pin;

use serde_json::json;
use tokio::sync::mpsc;

use crate::agent::event::AgentEvent;
use crate::agent::message::{AgentMessage, ToolCall};
use crate::agent::react::EngineState;
use crate::agent::tool::{ToolDefinition, ToolResult, str_arg};
use crate::provider::LlmProvider;

pub const TASK_TOOL: &str = "task";

pub fn task_definition() -> ToolDefinition {
    ToolDefinition {
        name: TASK_TOOL.into(),
        description: "Hand a self-contained task to a subagent with its own fresh context and the same tools. \
            Use it for broad searches or research whose details you do not need to keep. The subagent sees only \
            the prompt, so include every detail it needs; its final answer is returned to you."
            .into(),
        schema: json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "A few words naming the task" },
                "prompt": { "type": "string", "description": "The complete task for the subagent" }
            },
            "required": ["description", "prompt"]
        }),
    }
}

type TaskFuture<'a> = Pin<Box<dyn Future<Output = ToolResult> + Send + 'a>>;

impl<P: LlmProvider> EngineState<P> {
    /// The future is boxed and type-erased because the subagent's own loop contains this call.
    pub(super) fn run_task<'a>(&'a self, call: &'a ToolCall) -> TaskFuture<'a> {
        Box::pin(async move {
            let prompt = match str_arg(&call.input, "prompt") {
                Ok(prompt) => prompt.to_owned(),
                Err(e) => return ToolResult::err(e),
            };
            let (child, mut child_rx) = self.subagent(prompt);
            let mut run = Box::pin(child.into_history());
            let history = loop {
                tokio::select! {
                    history = &mut run => break history,
                    Some(event) = child_rx.recv() => self.forward(&call.id, event),
                }
            };
            while let Ok(event) = child_rx.try_recv() {
                self.forward(&call.id, event);
            }
            final_answer(&history).map_or_else(
                || ToolResult::err("The subagent stopped without an answer."),
                ToolResult::ok,
            )
        })
    }

    fn subagent(&self, prompt: String) -> (Self, mpsc::UnboundedReceiver<AgentEvent>) {
        let (agent_tx, child_rx) = mpsc::unbounded_channel();
        let mut child = EngineState {
            provider: self.provider.clone(),
            registry: self.registry.clone(),
            tools: self.registry.definitions(),
            system: self.system.clone(),
            history: Vec::new(),
            agent_tx,
            auto_approve: self.auto_approve,
            context_tokens: 0,
            depth: self.depth + 1,
        };
        child.commit(AgentMessage::User(prompt));
        (child, child_rx)
    }

    async fn into_history(mut self) -> Vec<AgentMessage> {
        self.react().await;
        self.history
    }

    fn forward(&self, call_id: &str, event: AgentEvent) {
        let line = match event {
            AgentEvent::Approval(request) => return self.emit(AgentEvent::Approval(request)),
            AgentEvent::ToolStart(call) => format!("{}: {}", call.name, call.summary()),
            AgentEvent::Notice(text) | AgentEvent::Error(text) => text,
            _ => return,
        };
        self.emit(AgentEvent::ToolOutput {
            call_id: call_id.to_owned(),
            line,
        });
    }
}

fn final_answer(history: &[AgentMessage]) -> Option<String> {
    match history.last()? {
        AgentMessage::Assistant(turn) if turn.tool_calls.is_empty() => turn.text.clone(),
        _ => None,
    }
}
