pub mod cli;
pub mod events;
pub mod feed;
pub mod input;
pub mod scroll;

pub use feed::{CommandOutputBlock, CommandStatus, FeedItem};
pub use input::{InputIntent, InputState, parse_input_intent};
pub use scroll::{DocPos, Mode, ScrollBounds, ScrollState, SelectionType};

use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::agent::{AgentConfig, AgentEvent, AgentHandle, AgentMessage, ToolRegistry, spawn_agent};
use crate::sys::ProcessHandle;

#[derive(Debug, Default, Clone)]
pub struct AppOptions {
    pub model_spec: Option<String>,
}

pub struct App {
    pub running: bool,
    pub mode: Mode,
    pub input: InputState,
    pub feed: Vec<FeedItem>,
    pub scroll: ScrollState,
    pub total_feed_lines: usize,
    pub viewport_height: usize,
    pub cursor: DocPos,
    pub selection_anchor: Option<DocPos>,
    pub selection_type: SelectionType,
    pub copied_flash: bool,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub draft_input: String,
    pub active_process: Option<ProcessHandle>,
    pub active_agent: Option<AgentHandle>,
    pub pending_agent_cmd: Option<String>,
    pub model_spec: String,
    pub agent_history: Vec<AgentMessage>,
}

impl App {
    pub fn new(options: AppOptions) -> Self {
        let model_spec = options
            .model_spec
            .or_else(|| std::env::var("PIRO_MODEL").ok())
            .unwrap_or_else(|| "ollama/ornith-1.5:35b".into());

        let mut app = Self {
            running: true,
            mode: Mode::Input,
            input: InputState::default(),
            feed: Vec::new(),
            scroll: ScrollState::default(),
            total_feed_lines: 0,
            viewport_height: 10,
            cursor: DocPos::default(),
            selection_anchor: None,
            selection_type: SelectionType::Char,
            copied_flash: false,
            history: Vec::new(),
            history_idx: None,
            draft_input: String::new(),
            active_process: None,
            active_agent: None,
            pending_agent_cmd: None,
            model_spec,
            agent_history: Vec::new(),
        };
        app.load_history();
        app
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new(AppOptions::default())
    }
}

impl App {
    fn history_file_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        let p = PathBuf::from(&home).join(".piro").join("history");
        let _ = std::fs::create_dir_all(p.parent()?);
        Some(p)
    }

    pub fn load_history(&mut self) {
        let Some(p) = Self::history_file_path() else { return };
        if let Ok(c) = std::fs::read_to_string(p) {
            for l in c.lines().map(str::trim).filter(|s| !s.is_empty()) {
                if self.history.last().map(|s| s.as_str()) != Some(l) {
                    if self.history.len() >= 1000 {
                        self.history.remove(0);
                    }
                    self.history.push(l.into());
                }
            }
        }
    }

    fn save_history(cmd: &str) {
        let Some(p) = Self::history_file_path() else { return };
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{}", cmd);
        }
    }

    pub fn is_command_running(&self) -> bool {
        self.active_process.is_some()
    }

    pub fn is_agent_running(&self) -> bool {
        self.active_agent.is_some()
    }

    pub fn is_busy(&self) -> bool {
        self.is_command_running() || self.is_agent_running()
    }

    pub fn record_history(&mut self, input: &str) {
        if self.history.last().map(|s| s.as_str()) != Some(input) {
            if self.history.len() >= 1000 {
                self.history.remove(0);
            }
            self.history.push(input.to_string());
            Self::save_history(input);
        }
        self.history_idx = None;
        self.draft_input.clear();
    }

    pub fn add_command(&mut self, cmd: String, h: ProcessHandle) {
        self.record_history(&cmd);
        self.feed.push(FeedItem::UserInput(cmd.clone()));
        self.feed
            .push(FeedItem::CommandOutput(CommandOutputBlock::new(cmd, None)));
        self.active_process = Some(h);
        self.scroll_to_bottom();
    }

    pub fn add_agent_prompt(&mut self, prompt: String, handle: AgentHandle) {
        self.record_history(&prompt);
        self.agent_history.push(AgentMessage::User(prompt.clone()));
        self.feed.push(FeedItem::UserInput(prompt));
        self.active_agent = Some(handle);
        self.scroll_to_bottom();
    }

    pub fn spawn_agent_prompt(&mut self, prompt: String, agent_tx: &mpsc::UnboundedSender<AgentEvent>) {
        let registry = ToolRegistry::with_defaults();
        let backend = crate::provider::create_backend(&self.model_spec, &registry)
            .unwrap_or_else(|_| crate::provider::default_backend(&registry))
            .with_delta_tx(agent_tx.clone());
        let config = AgentConfig {
            prompt: prompt.clone(),
            backend,
            registry,
            history: self.agent_history.clone(),
        };
        let handle = spawn_agent(config, agent_tx.clone());
        self.add_agent_prompt(prompt, handle);
    }

    pub fn clear_feed_and_agent_context(&mut self) {
        self.feed.clear();
        self.agent_history.clear();
        self.scroll_to_bottom();
    }

    fn append_slot(&mut self, is_thought: bool, text: String) {
        match self.feed.last_mut() {
            Some(FeedItem::AgentThought(t)) if is_thought => {
                t.push_str(&text);
                self.scroll_to_bottom();
                return;
            }
            Some(FeedItem::AgentResponse(r)) if !is_thought => {
                r.push_str(&text);
                self.scroll_to_bottom();
                return;
            }
            _ => {}
        }
        let item = if is_thought {
            FeedItem::AgentThought(text)
        } else {
            FeedItem::AgentResponse(text)
        };
        self.feed.push(item);
        self.scroll_to_bottom();
    }

    fn set_slot(&mut self, is_thought: bool, text: String) {
        for item in self.feed.iter_mut().rev() {
            match item {
                FeedItem::AgentThought(t) if is_thought => {
                    *t = text;
                    self.scroll_to_bottom();
                    return;
                }
                FeedItem::AgentResponse(r) if !is_thought => {
                    *r = text;
                    self.scroll_to_bottom();
                    return;
                }
                FeedItem::UserInput(_) => break,
                _ => {}
            }
        }
        let item = if is_thought {
            FeedItem::AgentThought(text)
        } else {
            FeedItem::AgentResponse(text)
        };
        self.feed.push(item);
        self.scroll_to_bottom();
    }

    pub fn append_agent_thought_delta(&mut self, delta: String) {
        self.append_slot(true, delta);
    }

    pub fn append_agent_response_delta(&mut self, delta: String) {
        self.append_slot(false, delta);
    }

    pub fn add_agent_thought(&mut self, thought: String) {
        self.set_slot(true, thought);
    }

    pub fn start_agent_tool(&mut self, name: String, input: String) {
        self.feed
            .push(FeedItem::CommandOutput(CommandOutputBlock::new(input, Some(name))));
        self.scroll_to_bottom();
    }

    pub fn add_agent_response(&mut self, response: String) {
        self.set_slot(false, response);
    }

    pub fn finish_agent(&mut self) {
        self.active_agent = None;
    }

    pub fn append_output_line(&mut self, line: String) {
        if let Some(FeedItem::CommandOutput(b)) = self.feed.last_mut() {
            if b.lines.len() >= 5000 {
                b.lines.remove(0);
            }
            b.lines.push(line);
        }
    }

    pub fn complete_command(&mut self, code: Option<i32>, dur: Duration) {
        self.active_process = None;
        if let Some(FeedItem::CommandOutput(b)) = self.feed.last_mut() {
            b.duration = Some(dur);
            b.status = match code {
                Some(0) => CommandStatus::Success(0),
                Some(c) => CommandStatus::Failed(c),
                None => CommandStatus::Failed(-1),
            };
        }
    }

    pub fn cancel_active(&mut self) -> bool {
        let mut cancelled = false;
        if let Some(agent) = self.active_agent.take() {
            agent.cancel();
            cancelled = true;
        }
        if let Some(mut h) = self.active_process.take() {
            h.kill();
            if let Some(FeedItem::CommandOutput(b)) = self.feed.last_mut() {
                b.duration = Some(b.started_at.elapsed());
                b.status = CommandStatus::Cancelled;
            }
            cancelled = true;
        }
        cancelled
    }

    pub fn enter_scroll_mode(&mut self) {
        self.mode = Mode::Scroll;
        self.clear_selection();
        self.copied_flash = false;
        self.cursor = DocPos {
            line: self.total_feed_lines.saturating_sub(1),
            col: 2,
        };
    }

    pub fn exit_scroll_mode(&mut self) {
        self.mode = Mode::Input;
        self.clear_selection();
    }

    pub fn toggle_selection(&mut self, t: SelectionType) {
        if self.selection_anchor.is_none() {
            self.selection_type = t;
            self.selection_anchor = Some(self.cursor);
        } else {
            self.clear_selection();
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
        self.selection_type = SelectionType::Char;
    }

    pub fn normalized_selection(&self) -> Option<(DocPos, DocPos)> {
        self.selection_anchor.map(|a| (a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn move_cursor_left(&mut self, d: u16) {
        self.cursor.col = self.cursor.col.saturating_sub(d);
    }

    pub fn move_cursor_right(&mut self, d: u16, max_col: u16) {
        self.cursor.col = (self.cursor.col + d).min(max_col);
    }

    pub fn move_cursor_up(&mut self, d: usize) {
        self.cursor.line = self.cursor.line.saturating_sub(d);
        let (start, _) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
        if self.cursor.line < start {
            self.scroll.scroll_up(
                start - self.cursor.line,
                ScrollBounds {
                    total: self.total_feed_lines,
                    view: self.viewport_height,
                },
            );
        }
    }

    pub fn move_cursor_down(&mut self, d: usize) -> bool {
        let max_l = self.total_feed_lines.saturating_sub(1);
        if self.cursor.line < max_l {
            self.cursor.line = (self.cursor.line + d).min(max_l);
            let (_, end) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
            if self.cursor.line >= end {
                self.scroll.scroll_down(self.cursor.line - end + 1);
            }
            false
        } else if self.scroll.scroll_from_bottom > 0 {
            self.scroll.scroll_down(d);
            false
        } else {
            true
        }
    }

    pub fn scroll_to_top(&mut self) {
        if self.total_feed_lines > self.viewport_height {
            self.scroll.scroll_from_bottom = self.total_feed_lines - self.viewport_height;
            self.scroll.auto_scroll = false;
        }
        self.cursor.line = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
        self.cursor.line = self.total_feed_lines.saturating_sub(1);
    }

    pub fn copy_current_block(&self) -> Option<String> {
        self.feed.iter().rev().find_map(|item| match item {
            FeedItem::CommandOutput(c) if !c.lines.is_empty() => Some(c.lines.join("\n")),
            FeedItem::AgentThought(t) if !t.is_empty() => Some(t.clone()),
            FeedItem::AgentResponse(r) if !r.is_empty() => Some(r.clone()),
            _ => None,
        })
    }

    pub fn toggle_expand(&mut self) {
        for item in self.feed.iter_mut().rev() {
            if let FeedItem::CommandOutput(b) = item {
                b.expanded = !b.expanded;
                break;
            }
        }
    }

    pub fn screen_coords_to_doc_pos(&self, col: u16, row: u16) -> Option<DocPos> {
        if self.total_feed_lines == 0 || self.viewport_height == 0 {
            return None;
        }
        let (start, end) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
        let count = end - start;
        let pad = if count < self.viewport_height && self.scroll.auto_scroll {
            self.viewport_height - count
        } else {
            0
        };
        let row = row as usize;
        if row < pad {
            return None;
        }
        Some(DocPos {
            line: (start + (row - pad)).min(self.total_feed_lines.saturating_sub(1)),
            col,
        })
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let i = match self.history_idx {
            None => {
                self.draft_input = self.input.value().into();
                self.history.len().saturating_sub(1)
            }
            Some(idx) => idx.saturating_sub(1),
        };
        self.history_idx = Some(i);
        self.input.set_value(self.history[i].clone());
    }

    pub fn history_next(&mut self) {
        let Some(i) = self.history_idx else { return };
        if i + 1 < self.history.len() {
            self.history_idx = Some(i + 1);
            self.input.set_value(self.history[i + 1].clone());
        } else {
            self.history_idx = None;
            self.input.set_value(std::mem::take(&mut self.draft_input));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_input_and_history() {
        let mut app = App::default();
        let h = ProcessHandle::mock(tokio::spawn(async {}).abort_handle());
        app.add_command("echo 1".into(), h);
        app.input.set_value("draft".into());
        app.history_prev();
        assert_eq!(app.input.value(), "echo 1");
        app.history_next();
        assert_eq!(app.input.value(), "draft");

        app.input.set_value("cargo test --release".into());
        app.input.delete_prev_word();
        assert_eq!(app.input.value(), "cargo test ");
    }

    #[test]
    fn test_scroll_and_selection() {
        let mut app = App::default();
        app.total_feed_lines = 50;
        app.viewport_height = 20;
        app.enter_scroll_mode();
        assert_eq!(app.cursor.line, 49);
        app.toggle_selection(SelectionType::Char);
        app.move_cursor_up(5);
        assert_eq!(app.cursor.line, 44);
        let (s, e) = app.normalized_selection().unwrap();
        assert_eq!((s.line, e.line), (44, 49));
    }

    #[tokio::test]
    async fn test_agent_history_accumulation_and_clear() {
        let mut app = App::default();
        assert!(app.agent_history.is_empty());
        let h = AgentHandle::mock(tokio::spawn(async {}).abort_handle());
        app.add_agent_prompt("test prompt".into(), h);
        assert_eq!(app.agent_history.len(), 1);
        assert_eq!(app.agent_history[0], AgentMessage::User("test prompt".into()));

        app.clear_feed_and_agent_context();
        assert!(app.agent_history.is_empty());
        assert!(app.feed.is_empty());
    }

    #[test]
    fn test_streaming_thought_and_response_does_not_duplicate() {
        let mut app = App::default();
        app.feed.push(FeedItem::UserInput("test prompt".into()));

        app.append_agent_thought_delta("Thinking part 1. ".into());
        app.append_agent_thought_delta("Thinking part 2.".into());

        app.append_agent_response_delta("Answer part 1. ".into());
        app.append_agent_response_delta("Answer part 2.".into());

        assert_eq!(app.feed.len(), 3);

        app.add_agent_thought("Thinking part 1. Thinking part 2.".into());
        app.add_agent_response("Answer part 1. Answer part 2.".into());

        assert_eq!(app.feed.len(), 3);
        match &app.feed[1] {
            FeedItem::AgentThought(t) => assert_eq!(t, "Thinking part 1. Thinking part 2."),
            _ => panic!("Expected AgentThought at index 1"),
        }
        match &app.feed[2] {
            FeedItem::AgentResponse(r) => assert_eq!(r, "Answer part 1. Answer part 2."),
            _ => panic!("Expected AgentResponse at index 2"),
        }
    }
}
