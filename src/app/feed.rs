use std::time::{Duration, Instant};

use crate::agent::tool::ToolExit;

const MAX_BLOCK_LINES: usize = 5000;
pub const TAIL_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandStatus {
    Running,
    Success(i32),
    Failed(i32),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct CommandOutputBlock {
    pub command: String,
    pub tool: Option<String>,
    pub call_id: Option<String>,
    pub status: CommandStatus,
    pub lines: Vec<String>,
    pub started_at: Instant,
    pub duration: Option<Duration>,
    pub expanded: bool,
}

impl CommandOutputBlock {
    pub fn new(command: String, tool: Option<String>) -> Self {
        Self {
            command,
            tool,
            call_id: None,
            status: CommandStatus::Running,
            lines: Vec::new(),
            started_at: Instant::now(),
            duration: None,
            expanded: false,
        }
    }

    pub fn push_line(&mut self, line: String) {
        if self.lines.len() >= MAX_BLOCK_LINES {
            self.lines.remove(0);
        }
        self.lines.push(line);
    }

    pub fn complete(&mut self, exit: ToolExit) {
        self.duration = Some(exit.duration);
        self.status = match exit.code {
            Some(0) => CommandStatus::Success(0),
            Some(c) => CommandStatus::Failed(c),
            None => CommandStatus::Failed(-1),
        };
    }

    pub fn cancel(&mut self) {
        self.duration = Some(self.started_at.elapsed());
        self.status = CommandStatus::Cancelled;
    }

    /// The lines to show, and how many lines above them are hidden.
    pub fn visible(&self) -> (usize, &[String]) {
        if self.expanded || self.lines.len() <= TAIL_LINES {
            (0, &self.lines)
        } else {
            let hidden = self.lines.len() - TAIL_LINES;
            (hidden, &self.lines[hidden..])
        }
    }

    pub fn copy_text(&self) -> String {
        self.lines.join("\n")
    }
}

#[derive(Debug, Clone)]
pub enum FeedItem {
    UserInput(String),
    CommandOutput(CommandOutputBlock),
    AgentThought(String),
    AgentResponse(String),
}

impl FeedItem {
    pub fn copy_text(&self) -> String {
        match self {
            Self::UserInput(t) | Self::AgentThought(t) | Self::AgentResponse(t) => t.clone(),
            Self::CommandOutput(b) => b.copy_text(),
        }
    }
}

/// Which streamed agent card a delta belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSlot {
    Thought,
    Response,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: u64,
    pub item: FeedItem,
}

/// The feed's cards in order. Ids are never reused, so a card keeps its identity while the feed changes.
#[derive(Debug, Default)]
pub struct Feed {
    entries: Vec<Entry>,
    next_id: u64,
}

impl Feed {
    pub fn push(&mut self, item: FeedItem) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(Entry { id, item });
        id
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn ids(&self) -> Vec<u64> {
        self.entries.iter().map(|e| e.id).collect()
    }

    pub fn get(&self, id: u64) -> Option<&FeedItem> {
        self.entries.iter().find(|e| e.id == id).map(|e| &e.item)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut FeedItem> {
        self.entries.iter_mut().find(|e| e.id == id).map(|e| &mut e.item)
    }

    pub fn command_mut(&mut self, id: u64) -> Option<&mut CommandOutputBlock> {
        match self.get_mut(id)? {
            FeedItem::CommandOutput(block) => Some(block),
            _ => None,
        }
    }

    pub fn latest_command_id(&self) -> Option<u64> {
        self.entries.iter().rev().find_map(|e| match e.item {
            FeedItem::CommandOutput(_) => Some(e.id),
            _ => None,
        })
    }

    pub fn tool_block_id(&self, call_id: &str) -> Option<u64> {
        self.entries.iter().rev().find_map(|e| match &e.item {
            FeedItem::CommandOutput(b) if b.call_id.as_deref() == Some(call_id) => Some(e.id),
            _ => None,
        })
    }

    /// Appends a streamed delta to the open card of `slot`, or starts a new one.
    pub fn append_delta(&mut self, slot: AgentSlot, delta: &str) {
        if let Some(Entry { item, .. }) = self.entries.last_mut()
            && let Some(text) = slot_text(item, slot)
        {
            text.push_str(delta);
            return;
        }
        self.push(slot_item(slot, delta.to_owned()));
    }

    /// Replaces the text of the latest `slot` card of this turn with the committed text, or adds one.
    pub fn set_text(&mut self, slot: AgentSlot, text: String) {
        for entry in self.entries.iter_mut().rev() {
            if matches!(entry.item, FeedItem::UserInput(_)) {
                break;
            }
            if let Some(current) = slot_text(&mut entry.item, slot) {
                *current = text;
                return;
            }
        }
        self.push(slot_item(slot, text));
    }
}

fn slot_text(item: &mut FeedItem, slot: AgentSlot) -> Option<&mut String> {
    match (item, slot) {
        (FeedItem::AgentThought(t), AgentSlot::Thought) => Some(t),
        (FeedItem::AgentResponse(r), AgentSlot::Response) => Some(r),
        _ => None,
    }
}

fn slot_item(slot: AgentSlot, text: String) -> FeedItem {
    match slot {
        AgentSlot::Thought => FeedItem::AgentThought(text),
        AgentSlot::Response => FeedItem::AgentResponse(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_with_lines(n: usize) -> CommandOutputBlock {
        let mut block = CommandOutputBlock::new("seq".into(), None);
        for i in 1..=n {
            block.push_line(format!("line {i}"));
        }
        block
    }

    #[test]
    fn output_collapses_to_the_last_lines() {
        let mut block = block_with_lines(20);
        let (hidden, shown) = block.visible();
        assert_eq!(hidden, 20 - TAIL_LINES);
        assert_eq!(shown.first().map(String::as_str), Some("line 13"));
        assert_eq!(shown.len(), TAIL_LINES);

        block.expanded = true;
        assert_eq!(block.visible().0, 0);
        assert_eq!(block.visible().1.len(), 20);
    }

    #[test]
    fn short_output_is_not_collapsed() {
        let block = block_with_lines(TAIL_LINES);
        assert_eq!(block.visible().0, 0);
    }

    #[test]
    fn ids_are_never_reused_after_clear() {
        let mut feed = Feed::default();
        let first = feed.push(FeedItem::UserInput("a".into()));
        feed.clear();
        let second = feed.push(FeedItem::UserInput("b".into()));
        assert_ne!(first, second);
        assert_eq!(feed.ids(), [second]);
    }

    #[test]
    fn latest_command_is_the_newest_block() {
        let mut feed = Feed::default();
        feed.push(FeedItem::CommandOutput(block_with_lines(1)));
        let newest = feed.push(FeedItem::CommandOutput(block_with_lines(1)));
        feed.push(FeedItem::AgentResponse("done".into()));
        assert_eq!(feed.latest_command_id(), Some(newest));
    }
}
