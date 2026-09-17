pub mod cli;
pub mod events;

use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::agent::{AgentHandle, AgentMessage};
use crate::sys::ProcessHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputIntent {
    DirectCommand(String),
    CommandWithAgent(String),
    AgentPrompt(String),
}

pub fn parse_input_intent(input: &str) -> Option<InputIntent> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(cmd) = trimmed.strip_prefix("!!") {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            return Some(InputIntent::DirectCommand(cmd.to_string()));
        }
    } else if let Some(cmd) = trimmed.strip_prefix('!') {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            return Some(InputIntent::CommandWithAgent(cmd.to_string()));
        }
    }
    Some(InputIntent::AgentPrompt(trimmed.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Input,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionType {
    #[default]
    Char,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DocPos {
    pub line: usize,
    pub col: u16,
}

impl PartialOrd for DocPos {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for DocPos {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.line, self.col).cmp(&(o.line, o.col))
    }
}

#[derive(Debug, Default, Clone)]
pub struct InputState {
    value: String,
    cursor: usize,
}

impl InputState {
    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn line_count(&self) -> usize {
        self.value.split('\n').count()
    }

    pub fn cursor_coords(&self) -> (usize, usize) {
        let mut row = 0;
        let mut col = 0;
        for (idx, c) in self.value.chars().enumerate() {
            if idx == self.cursor {
                return (col, row);
            }
            if c == '\n' {
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (col, row)
    }

    pub fn move_up(&mut self) -> bool {
        let (cur_col, cur_row) = self.cursor_coords();
        if cur_row == 0 {
            return false;
        }
        self.cursor = self.char_idx_for_coords(cur_col, cur_row - 1);
        true
    }

    pub fn move_down(&mut self) -> bool {
        let (cur_col, cur_row) = self.cursor_coords();
        let total_lines = self.line_count();
        if cur_row + 1 >= total_lines {
            return false;
        }
        self.cursor = self.char_idx_for_coords(cur_col, cur_row + 1);
        true
    }

    fn char_idx_for_coords(&self, col: usize, target_row: usize) -> usize {
        let mut char_idx = 0;
        for (row, line) in self.value.split('\n').enumerate() {
            let line_len = line.chars().count();
            if row == target_row {
                return char_idx + col.min(line_len);
            }
            char_idx += line_len + 1;
        }
        self.value.chars().count()
    }

    fn byte_idx(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let i = self.byte_idx();
        self.value.insert(i, c);
        self.cursor += 1;
    }

    pub fn delete_back(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let i = self.byte_idx();
            self.value.remove(i);
        }
    }

    pub fn delete_forward(&mut self) {
        if self.cursor < self.value.chars().count() {
            let i = self.byte_idx();
            self.value.remove(i);
        }
    }

    pub fn delete_prev_word(&mut self) {
        let b = self.byte_idx();
        let s = &self.value[..b];
        let i = s.trim_end().rfind(char::is_whitespace).map_or(0, |x| x + 1);
        self.value.drain(i..b);
        self.cursor = self.value[..i].chars().count();
    }

    pub fn delete_line(&mut self) {
        let i = self.byte_idx();
        self.value.drain(..i);
        self.cursor = 0;
    }

    pub fn delete_to_end(&mut self) {
        let i = self.byte_idx();
        self.value.truncate(i);
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn move_right(&mut self) {
        if self.cursor < self.value.chars().count() {
            self.cursor += 1;
        }
    }
    pub fn move_start(&mut self) {
        self.cursor = 0;
    }
    pub fn move_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    pub fn move_prev_word(&mut self) {
        let s = &self.value[..self.byte_idx()];
        let i = s.trim_end().rfind(char::is_whitespace).map_or(0, |x| x + 1);
        self.cursor = self.value[..i].chars().count();
    }

    pub fn move_next_word(&mut self) {
        let s = &self.value[self.byte_idx()..];
        let trimmed = s.trim_start();
        let skip = s.len() - trimmed.len();
        let w = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        self.cursor += s[..skip + w].chars().count();
    }

    pub fn set_value(&mut self, val: String) {
        self.cursor = val.chars().count();
        self.value = val;
    }
    pub fn take_value(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }
}

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
    pub status: CommandStatus,
    pub lines: Vec<String>,
    pub started_at: Instant,
    pub duration: Option<Duration>,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub enum FeedItem {
    UserInput(String),
    CommandOutput(CommandOutputBlock),
    AgentThought(String),
    AgentResponse(String),
}

#[derive(Debug, Clone, Copy)]
pub struct ScrollBounds {
    pub total: usize,
    pub view: usize,
}

#[derive(Debug, Default, Clone)]
pub struct ScrollState {
    pub scroll_from_bottom: usize,
    pub auto_scroll: bool,
}

impl ScrollState {
    pub fn scroll_up(&mut self, delta: usize, bounds: ScrollBounds) {
        if bounds.total > bounds.view {
            self.scroll_from_bottom = (self.scroll_from_bottom + delta).min(bounds.total - bounds.view);
            self.auto_scroll = false;
        }
    }

    pub fn scroll_down(&mut self, delta: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(delta);
        if self.scroll_from_bottom == 0 {
            self.auto_scroll = true;
        }
    }

    pub fn visible_range(&self, total: usize, view: usize) -> (usize, usize) {
        if total <= view {
            return (0, total);
        }
        let max = total - view;
        let offset = if self.auto_scroll {
            0
        } else {
            self.scroll_from_bottom.min(max)
        };
        let start = max - offset;
        (start, (start + view).min(total))
    }
}

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
        self.feed.push(FeedItem::CommandOutput(CommandOutputBlock {
            command: cmd,
            tool: None,
            status: CommandStatus::Running,
            lines: Vec::new(),
            started_at: Instant::now(),
            duration: None,
            expanded: false,
        }));
        self.active_process = Some(h);
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
    }

    pub fn add_agent_prompt(&mut self, prompt: String, handle: AgentHandle) {
        self.record_history(&prompt);
        self.agent_history.push(AgentMessage::User(prompt.clone()));
        self.feed.push(FeedItem::UserInput(prompt));
        self.active_agent = Some(handle);
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
    }

    pub fn clear_feed_and_agent_context(&mut self) {
        self.feed.clear();
        self.agent_history.clear();
        self.scroll_to_bottom();
    }

    pub fn append_agent_thought_delta(&mut self, delta: String) {
        if let Some(FeedItem::AgentThought(text)) = self.feed.last_mut() {
            text.push_str(&delta);
        } else {
            self.feed.push(FeedItem::AgentThought(delta));
        }
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
    }

    pub fn append_agent_response_delta(&mut self, delta: String) {
        if let Some(FeedItem::AgentResponse(text)) = self.feed.last_mut() {
            text.push_str(&delta);
        } else {
            self.feed.push(FeedItem::AgentResponse(delta));
        }
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
    }

    pub fn add_agent_thought(&mut self, thought: String) {
        if let Some(FeedItem::AgentThought(text)) = self.feed.last_mut() {
            *text = thought;
        } else {
            self.feed.push(FeedItem::AgentThought(thought));
        }
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
    }

    pub fn start_agent_tool(&mut self, name: String, input: String) {
        self.feed.push(FeedItem::CommandOutput(CommandOutputBlock {
            command: input,
            tool: Some(name),
            status: CommandStatus::Running,
            lines: Vec::new(),
            started_at: Instant::now(),
            duration: None,
            expanded: false,
        }));
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
    }

    pub fn add_agent_response(&mut self, response: String) {
        if let Some(FeedItem::AgentResponse(text)) = self.feed.last_mut() {
            *text = response;
        } else {
            self.feed.push(FeedItem::AgentResponse(response));
        }
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
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
        self.selection_anchor = None;
        self.selection_type = SelectionType::Char;
        self.copied_flash = false;
        self.cursor = DocPos {
            line: self.total_feed_lines.saturating_sub(1),
            col: 2,
        };
    }

    pub fn exit_scroll_mode(&mut self) {
        self.mode = Mode::Input;
        self.selection_anchor = None;
        self.selection_type = SelectionType::Char;
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
        self.selection_anchor.map(|a| {
            if a <= self.cursor {
                (a, self.cursor)
            } else {
                (self.cursor, a)
            }
        })
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

    #[test]
    fn test_parse_input_intent() {
        assert_eq!(parse_input_intent(""), None);
        assert_eq!(parse_input_intent("   "), None);
        assert_eq!(
            parse_input_intent("!!ls -la"),
            Some(InputIntent::DirectCommand("ls -la".into()))
        );
        assert_eq!(
            parse_input_intent("!git status"),
            Some(InputIntent::CommandWithAgent("git status".into()))
        );
        assert_eq!(
            parse_input_intent("How many files are in this directory?"),
            Some(InputIntent::AgentPrompt("How many files are in this directory?".into()))
        );
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
    fn test_input_cursor_coords() {
        let mut input = InputState::default();
        assert_eq!(input.line_count(), 1);
        assert_eq!(input.cursor_coords(), (0, 0));

        input.set_value("first\nsecond\nthird".into());
        assert_eq!(input.line_count(), 3);
        // set_value puts cursor at the very end (col 5, row 2)
        assert_eq!(input.cursor_coords(), (5, 2));

        input.move_start();
        assert_eq!(input.cursor_coords(), (0, 0));

        // Move to right 5 times -> end of "first"
        for _ in 0..5 {
            input.move_right();
        }
        assert_eq!(input.cursor_coords(), (5, 0));

        // Move right 1 more -> starts "second"
        input.move_right();
        assert_eq!(input.cursor_coords(), (0, 1));
    }

    #[test]
    fn test_input_vertical_movement() {
        let mut input = InputState::default();
        input.set_value("short\nlonger line\nabc".into());

        // Cursor starts at end: "abc", row 2, col 3
        assert_eq!(input.cursor_coords(), (3, 2));

        // Move down when already on bottom row -> returns false
        assert!(!input.move_down());
        assert_eq!(input.cursor_coords(), (3, 2));

        // Move up to "longer line" -> row 1, col 3
        assert!(input.move_up());
        assert_eq!(input.cursor_coords(), (3, 1));

        // Move to end of "longer line" (col 11, row 1)
        for _ in 0..8 {
            input.move_right();
        }
        assert_eq!(input.cursor_coords(), (11, 1));

        // Move up to "short" (length 5) -> clamps to col 5, row 0
        assert!(input.move_up());
        assert_eq!(input.cursor_coords(), (5, 0));

        // Move up when already on top row -> returns false
        assert!(!input.move_up());
        assert_eq!(input.cursor_coords(), (5, 0));

        // Move down back to "longer line" -> row 1, col 5
        assert!(input.move_down());
        assert_eq!(input.cursor_coords(), (5, 1));

        // Move down to "abc" (length 3) -> clamps to col 3, row 2
        assert!(input.move_down());
        assert_eq!(input.cursor_coords(), (3, 2));
    }
}
