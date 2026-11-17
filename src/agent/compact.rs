use crate::agent::event::AgentEvent;
use crate::agent::message::AgentMessage;
use crate::agent::react::EngineState;
use crate::provider::LlmProvider;

const DEFAULT_THRESHOLD: u64 = 160_000;
const PROMPT: &str = "Summarize this conversation so that work can continue from the summary alone: the user's \
goals and constraints, what was done and learned (files, commands, decisions), the current state, and the next \
steps. Reply with the summary only and do not call tools.";
const SUMMARY_PREFIX: &str = "This session continues from an earlier conversation, summarized here:";

pub fn threshold() -> u64 {
    std::env::var("PIRO_COMPACT_AT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_THRESHOLD)
}

impl<P: LlmProvider> EngineState<P> {
    pub(super) async fn compact_if_full(&mut self) {
        if self.context_tokens >= threshold() {
            self.compact().await;
        }
    }

    pub(super) async fn compact(&mut self) {
        self.context_tokens = 0;
        self.emit(AgentEvent::Notice("Compacting the conversation...".into()));
        self.history.push(AgentMessage::User(PROMPT.into()));
        let mut params = self.params();
        params.delta_tx = None;
        let summary = self.provider.chat_stream(params).await;
        self.history.pop();
        match summary.map(|reply| reply.turn.text) {
            Ok(Some(summary)) => {
                self.history = vec![AgentMessage::User(format!("{SUMMARY_PREFIX}\n\n{summary}"))];
                self.emit(AgentEvent::Compacted(self.history.clone()));
            }
            Ok(None) => self.emit(AgentEvent::Notice("Compaction failed: the summary was empty.".into())),
            Err(e) => self.emit(AgentEvent::Notice(format!("Compaction failed: {}", e.message))),
        }
    }
}
