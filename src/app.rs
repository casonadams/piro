use std::time::{Duration, Instant};
use tokio::task::AbortHandle;

#[derive(Debug, Default, Clone)]
pub struct InputState {
    value: String,
    cursor: usize, // character index
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
pub struct UserInputBlock {
    pub command: String,
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
    UserInput(UserInputBlock),
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
    pub input: InputState,
    pub feed: Vec<FeedItem>,
    pub scroll: ScrollState,
    pub total_feed_lines: usize,
    pub viewport_height: usize,
    pub active_abort: Option<AbortHandle>,
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            input: InputState::default(),
            feed: Vec::new(),
            scroll: ScrollState::new(),
            total_feed_lines: 0,
            viewport_height: 10,
            active_abort: None,
        }
    }

    pub fn is_command_running(&self) -> bool {
        self.active_abort.is_some()
    }

    pub fn add_command(&mut self, command: String, handle: AbortHandle) {
        self.feed.push(FeedItem::UserInput(UserInputBlock {
            command: command.clone(),
        }));
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

    pub fn scroll_up(&mut self, delta: usize) {
        self.scroll
            .scroll_up(delta, self.total_feed_lines, self.viewport_height);
    }

    pub fn scroll_down(&mut self, delta: usize) {
        self.scroll.scroll_down(delta);
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
}
