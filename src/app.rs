use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::runner::ProcessHandle;

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
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DocPos {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.line, self.col).cmp(&(other.line, other.col))
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

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn byte_idx(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let idx = self.byte_idx();
        self.value.insert(idx, c);
        self.cursor += 1;
    }

    pub fn delete_back(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let idx = self.byte_idx();
            self.value.remove(idx);
        }
    }

    pub fn delete_forward(&mut self) {
        if self.cursor < self.value.chars().count() {
            let idx = self.byte_idx();
            self.value.remove(idx);
        }
    }

    pub fn delete_prev_word(&mut self) {
        let val = &self.value;
        if self.cursor == 0 || val.is_empty() {
            return;
        }

        let chars: Vec<char> = val.chars().collect();
        let cur = self.cursor.min(chars.len());
        let mut idx = cur;

        // Skip any whitespace immediately before cursor
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        // Delete backward across non-whitespace characters (bash unix-word-rubout)
        while idx > 0 && !chars[idx - 1].is_whitespace() {
            idx -= 1;
        }

        let mut new_val: String = chars[..idx].iter().collect();
        if cur < chars.len() {
            new_val.extend(chars[cur..].iter());
        }
        self.value = new_val;
        self.cursor = idx;
    }

    pub fn delete_line(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let cur = self.cursor.min(chars.len());
        self.value = chars[cur..].iter().collect();
        self.cursor = 0;
    }

    pub fn delete_to_end(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let cur = self.cursor.min(chars.len());
        self.value = chars[..cur].iter().collect();
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
        let chars: Vec<char> = self.value.chars().collect();
        let mut idx = self.cursor.min(chars.len());
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        while idx > 0 && !chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        self.cursor = idx;
    }

    pub fn move_next_word(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        let mut idx = self.cursor.min(len);
        while idx < len && !chars[idx].is_whitespace() {
            idx += 1;
        }
        while idx < len && chars[idx].is_whitespace() {
            idx += 1;
        }
        self.cursor = idx;
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
    pub status: CommandStatus,
    pub lines: Vec<String>,
    pub started_at: Instant,
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone)]
pub enum FeedItem {
    UserInput(String),
    CommandOutput(CommandOutputBlock),
}

#[derive(Debug, Default, Clone)]
pub struct ScrollState {
    pub scroll_from_bottom: usize,
    pub auto_scroll: bool,
}

impl ScrollState {
    pub fn scroll_up(&mut self, delta: usize, total: usize, view: usize) {
        if total > view {
            self.scroll_from_bottom = (self.scroll_from_bottom + delta).min(total - view);
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
}

impl App {
    pub fn new() -> Self {
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
        };
        app.load_history();
        app
    }

    pub fn piro_dir() -> PathBuf {
        let local = PathBuf::from(".piro");
        if local.is_dir() {
            return local;
        }

        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".piro")
    }

    pub fn history_file_path() -> PathBuf {
        let dir = Self::piro_dir();
        let _ = std::fs::create_dir_all(&dir);
        let history_file = dir.join("history");

        if !history_file.exists() {
            let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
                return history_file;
            };
            let legacy_piro = PathBuf::from(&home).join(".piro_history");
            let legacy_tui = PathBuf::from(&home).join(".tuitest_history");
            if legacy_piro.exists() {
                let _ = std::fs::rename(&legacy_piro, &history_file);
            } else if legacy_tui.exists() {
                let _ = std::fs::rename(&legacy_tui, &history_file);
            }
        }

        history_file
    }

    pub fn load_history(&mut self) {
        let path = Self::history_file_path();
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && self.history.last().map(|s| s.as_str()) != Some(trimmed) {
                if self.history.len() >= 1000 {
                    self.history.remove(0);
                }
                self.history.push(trimmed.to_string());
            }
        }
    }

    fn save_history_entry(cmd: &str) {
        let path = Self::history_file_path();
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{}", cmd);
        }
    }

    pub fn is_command_running(&self) -> bool {
        self.active_process.is_some()
    }

    pub fn add_command(&mut self, command: String, handle: ProcessHandle) {
        if self.history.last().map(|s| s.as_str()) != Some(&command) {
            if self.history.len() >= 1000 {
                self.history.remove(0);
            }
            self.history.push(command.clone());
            Self::save_history_entry(&command);
        }
        self.history_idx = None;
        self.draft_input.clear();
        self.feed.push(FeedItem::UserInput(command.clone()));
        self.feed.push(FeedItem::CommandOutput(CommandOutputBlock {
            command,
            status: CommandStatus::Running,
            lines: Vec::new(),
            started_at: Instant::now(),
            duration: None,
        }));
        self.active_process = Some(handle);
        self.scroll.scroll_from_bottom = 0;
        self.scroll.auto_scroll = true;
    }

    pub fn append_output_line(&mut self, line: String) {
        if let Some(FeedItem::CommandOutput(b)) = self.feed.last_mut() {
            if b.lines.len() >= 5000 {
                b.lines.remove(0);
            }
            b.lines.push(line);
        }
    }

    pub fn complete_command(&mut self, exit_code: Option<i32>, duration: Duration) {
        self.active_process = None;
        if let Some(FeedItem::CommandOutput(b)) = self.feed.last_mut() {
            b.duration = Some(duration);
            b.status = match exit_code {
                Some(0) => CommandStatus::Success(0),
                Some(c) => CommandStatus::Failed(c),
                None => CommandStatus::Failed(-1),
            };
        }
    }

    pub fn cancel_active(&mut self) -> bool {
        if let Some(mut handle) = self.active_process.take() {
            handle.kill();
            if let Some(FeedItem::CommandOutput(b)) = self.feed.last_mut() {
                b.duration = Some(b.started_at.elapsed());
                b.status = CommandStatus::Cancelled;
            }
            true
        } else {
            false
        }
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

    pub fn toggle_selection(&mut self, sel_type: SelectionType) {
        if self.selection_anchor.is_none() {
            self.selection_type = sel_type;
            self.selection_anchor = Some(self.cursor);
        } else {
            self.selection_anchor = None;
            self.selection_type = SelectionType::Char;
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

    pub fn move_cursor_left(&mut self, delta: u16) {
        self.cursor.col = self.cursor.col.saturating_sub(delta);
    }

    pub fn move_cursor_right(&mut self, delta: u16, max_col: u16) {
        self.cursor.col = (self.cursor.col + delta).min(max_col);
    }

    pub fn move_cursor_up(&mut self, delta: usize) {
        self.cursor.line = self.cursor.line.saturating_sub(delta);
        let (start, _) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
        if self.cursor.line < start {
            self.scroll
                .scroll_up(start - self.cursor.line, self.total_feed_lines, self.viewport_height);
        }
    }

    pub fn move_cursor_down(&mut self, delta: usize) -> bool {
        let max_line = self.total_feed_lines.saturating_sub(1);
        if self.cursor.line < max_line {
            self.cursor.line = (self.cursor.line + delta).min(max_line);
            let (_, end) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
            if self.cursor.line >= end {
                self.scroll.scroll_down(self.cursor.line - end + 1);
            }
            false
        } else if self.scroll.scroll_from_bottom > 0 {
            self.scroll_down(delta);
            false
        } else {
            true
        }
    }

    pub fn scroll_down(&mut self, delta: usize) {
        self.scroll.scroll_down(delta);
        let (start, end) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
        self.cursor.line = self.cursor.line.clamp(start, end.saturating_sub(1));
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
            FeedItem::CommandOutput(cmd) if !cmd.lines.is_empty() => Some(cmd.lines.join("\n")),
            _ => None,
        })
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
        let next_idx = match self.history_idx {
            None => {
                self.draft_input = self.input.value().to_string();
                self.history.len().saturating_sub(1)
            }
            Some(i) => i.saturating_sub(1),
        };
        self.history_idx = Some(next_idx);
        self.input.set_value(self.history[next_idx].clone());
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

    #[test]
    fn test_input_state_editing() {
        let mut input = InputState::default();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        assert_eq!(input.value(), "abc");
        assert_eq!(input.cursor(), 3);

        input.move_left();
        input.delete_back();
        assert_eq!(input.value(), "ac");

        input.move_start();
        input.insert('z');
        assert_eq!(input.value(), "zac");

        // Test delete_prev_word (unix-word-rubout)
        input.set_value("cargo test --release".into());
        input.delete_prev_word();
        assert_eq!(input.value(), "cargo test ");

        input.delete_prev_word();
        assert_eq!(input.value(), "cargo ");

        // Test delete_line (Ctrl+U)
        input.set_value("hello world".into());
        input.delete_line();
        assert_eq!(input.value(), "");
    }

    #[tokio::test]
    async fn test_input_and_history() {
        let mut app = App::new();
        let handle = ProcessHandle::mock(tokio::spawn(async {}).abort_handle());
        app.add_command("echo 1".into(), handle);
        app.input.set_value("draft".into());
        app.history_prev();
        assert_eq!(app.input.value(), "echo 1");
        app.history_next();
        assert_eq!(app.input.value(), "draft");
    }

    #[test]
    fn test_scroll_and_selection() {
        let mut app = App::new();
        app.total_feed_lines = 50;
        app.viewport_height = 20;

        app.enter_scroll_mode();
        assert_eq!(app.cursor.line, 49);
        app.toggle_selection(SelectionType::Char);
        app.move_cursor_up(5);
        assert_eq!(app.cursor.line, 44);
        let (s, e) = app.normalized_selection().unwrap();
        assert_eq!((s.line, e.line), (44, 49));

        assert!(!app.move_cursor_down(5));
        assert!(app.move_cursor_down(1));
    }

    #[test]
    fn test_piro_paths() {
        let dir = App::piro_dir();
        assert!(dir.to_string_lossy().contains(".piro"));
        let hist = App::history_file_path();
        assert!(hist.to_string_lossy().ends_with("history"));
    }
}
