use std::cmp::Ordering;
use std::time::{Duration, Instant};
use tokio::task::AbortHandle;

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

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let idx = self.byte_index();
        self.value.insert(idx, c);
        self.cursor += 1;
    }

    pub fn delete_back(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let idx = self.byte_index();
            self.value.remove(idx);
        }
    }

    pub fn delete_forward(&mut self) {
        if self.cursor < self.value.chars().count() {
            let idx = self.byte_index();
            self.value.remove(idx);
        }
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
    pub fn new() -> Self {
        Self {
            scroll_from_bottom: 0,
            auto_scroll: true,
        }
    }

    pub fn scroll_up(&mut self, delta: usize, total_lines: usize, viewport_height: usize) {
        if total_lines <= viewport_height {
            return;
        }
        let max = total_lines.saturating_sub(viewport_height);
        self.scroll_from_bottom = (self.scroll_from_bottom + delta).min(max);
        self.auto_scroll = false;
    }

    pub fn scroll_down(&mut self, delta: usize) {
        if self.scroll_from_bottom <= delta {
            self.scroll_from_bottom = 0;
            self.auto_scroll = true;
        } else {
            self.scroll_from_bottom -= delta;
        }
    }

    pub fn scroll_to_top(&mut self, total_lines: usize, viewport_height: usize) {
        if total_lines > viewport_height {
            self.scroll_from_bottom = total_lines - viewport_height;
            self.auto_scroll = false;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_from_bottom = 0;
        self.auto_scroll = true;
    }

    pub fn visible_range(&self, total_lines: usize, viewport_height: usize) -> (usize, usize) {
        if total_lines <= viewport_height {
            return (0, total_lines);
        }
        let max = total_lines.saturating_sub(viewport_height);
        let offset = if self.auto_scroll {
            0
        } else {
            self.scroll_from_bottom.min(max)
        };
        let start = max - offset;
        (start, (start + viewport_height).min(total_lines))
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
    pub active_abort: Option<AbortHandle>,
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            mode: Mode::Input,
            input: InputState::default(),
            feed: Vec::new(),
            scroll: ScrollState::new(),
            total_feed_lines: 0,
            viewport_height: 10,
            cursor: DocPos::default(),
            selection_anchor: None,
            selection_type: SelectionType::Char,
            copied_flash: false,
            active_abort: None,
        }
    }

    pub fn is_command_running(&self) -> bool {
        self.active_abort.is_some()
    }

    pub fn add_command(&mut self, command: String, handle: AbortHandle) {
        self.feed.push(FeedItem::UserInput(command.clone()));
        self.feed.push(FeedItem::CommandOutput(CommandOutputBlock {
            command,
            status: CommandStatus::Running,
            lines: Vec::new(),
            started_at: Instant::now(),
            duration: None,
        }));
        self.active_abort = Some(handle);
        self.scroll.scroll_to_bottom();
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
        self.active_abort = None;
        if let Some(FeedItem::CommandOutput(b)) = self.feed.last_mut() {
            b.duration = Some(duration);
            b.status = match exit_code {
                Some(0) => CommandStatus::Success(0),
                Some(code) => CommandStatus::Failed(code),
                None => CommandStatus::Failed(-1),
            };
        }
    }

    pub fn cancel_active(&mut self) -> bool {
        if let Some(handle) = self.active_abort.take() {
            handle.abort();
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
        let line = self.total_feed_lines.saturating_sub(1);
        self.cursor = DocPos { line, col: 2 };
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
        self.selection_anchor.map(|anchor| {
            if anchor <= self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
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
        let (start, _end) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
        if self.cursor.line < start {
            self.scroll
                .scroll_up(start - self.cursor.line, self.total_feed_lines, self.viewport_height);
        }
    }

    pub fn move_cursor_down(&mut self, delta: usize) -> bool {
        let max_line = self.total_feed_lines.saturating_sub(1);
        if self.cursor.line < max_line {
            self.cursor.line = (self.cursor.line + delta).min(max_line);
            let (_start, end) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
            if self.cursor.line >= end {
                self.scroll.scroll_down(self.cursor.line - end + 1);
            }
            false
        } else if self.scroll.scroll_from_bottom > 0 {
            self.scroll_down(delta);
            false
        } else {
            // Already at the bottom line of the document and at bottom of viewport
            true
        }
    }

    pub fn scroll_down(&mut self, delta: usize) {
        self.scroll.scroll_down(delta);
        let (start, end) = self.scroll.visible_range(self.total_feed_lines, self.viewport_height);
        self.cursor.line = self.cursor.line.clamp(start, end.saturating_sub(1));
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll.scroll_to_top(self.total_feed_lines, self.viewport_height);
        self.cursor.line = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll.scroll_to_bottom();
        self.cursor.line = self.total_feed_lines.saturating_sub(1);
    }

    pub fn copy_current_block(&self) -> Option<String> {
        for item in self.feed.iter().rev() {
            if let FeedItem::CommandOutput(cmd) = item {
                if cmd.lines.is_empty() {
                    continue;
                }
                return Some(cmd.lines.join("\n"));
            }
        }
        None
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
        let line = (start + (row - pad)).min(self.total_feed_lines.saturating_sub(1));
        Some(DocPos { line, col })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_state() {
        let mut input = InputState::default();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        assert_eq!(input.value(), "abc");
        assert_eq!(input.cursor(), 3);

        input.move_left();
        input.delete_back();
        assert_eq!(input.value(), "ac");

        input.delete_forward();
        assert_eq!(input.value(), "a");
        assert_eq!(input.take_value(), "a");
        assert_eq!(input.value(), "");
    }

    #[test]
    fn test_scroll_state() {
        let mut scroll = ScrollState::new();
        assert_eq!(scroll.visible_range(100, 20), (80, 100));

        scroll.scroll_up(10, 100, 20);
        assert_eq!(scroll.visible_range(100, 20), (70, 90));

        scroll.scroll_down(10);
        assert_eq!(scroll.visible_range(100, 20), (80, 100));
        assert!(scroll.auto_scroll);
    }

    #[test]
    fn test_cursor_and_selection() {
        let mut app = App::new();
        app.total_feed_lines = 50;
        app.viewport_height = 20;

        app.enter_scroll_mode();
        assert_eq!(app.mode, Mode::Scroll);
        assert_eq!(app.cursor.line, 49);
        assert_eq!(app.selection_anchor, None);

        // Character visual selection
        app.toggle_selection(SelectionType::Char);
        assert_eq!(app.selection_anchor, Some(DocPos { line: 49, col: 2 }));

        app.move_cursor_up(5);
        assert_eq!(app.cursor.line, 44);

        let (start, end) = app.normalized_selection().unwrap();
        assert_eq!(start.line, 44);
        assert_eq!(end.line, 49);

        // Line visual selection (Shift+V)
        app.toggle_selection(SelectionType::Line); // cancels current selection
        app.toggle_selection(SelectionType::Line);
        assert_eq!(app.selection_type, SelectionType::Line);

        app.clear_selection();
        assert_eq!(app.selection_anchor, None);
        assert_eq!(app.normalized_selection(), None);

        app.exit_scroll_mode();
        assert_eq!(app.mode, Mode::Input);
    }

    #[test]
    fn test_scroll_down_to_bottom_before_exit() {
        let mut app = App::new();
        app.total_feed_lines = 50;
        app.viewport_height = 20;

        app.enter_scroll_mode();
        app.move_cursor_up(10);
        assert_eq!(app.cursor.line, 39);

        let past = app.move_cursor_down(5);
        assert!(!past);
        assert_eq!(app.cursor.line, 44);

        let past = app.move_cursor_down(10);
        assert!(!past);
        assert_eq!(app.cursor.line, 49);

        // Now scrolling down past the bottom-most line signals exit!
        let past = app.move_cursor_down(2);
        assert!(past);
    }
}
