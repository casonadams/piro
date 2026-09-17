use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputIntent {
    DirectCommand(String),
    CommandWithAgent(String),
    AgentPrompt(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualCoords {
    pub col: usize,
    pub row: usize,
    pub width: usize,
}

struct CoordFinder {
    target: VisualCoords,
    row: usize,
    col: usize,
    best_char_idx: usize,
}

impl CoordFinder {
    fn new(target: VisualCoords) -> Self {
        Self {
            target,
            row: 0,
            col: 0,
            best_char_idx: 0,
        }
    }

    fn step_char(&mut self, idx: usize, c: char) -> bool {
        if c == '\n' {
            if self.row == self.target.row {
                self.best_char_idx = idx;
                return true;
            }
            self.row += 1;
            self.col = 0;
            return false;
        }

        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        let target_w = self.target.width.max(1);
        if self.col > 0 && self.col + cw > target_w {
            if self.row == self.target.row {
                return true;
            }
            self.row += 1;
            self.col = 0;
        }

        if self.row == self.target.row {
            self.best_char_idx = idx;
            if self.col + cw > self.target.col {
                return true;
            }
        }
        self.col += cw;
        false
    }

    fn finish(self, text_len: usize) -> usize {
        if self.row == self.target.row && self.target.col >= self.col {
            return text_len;
        }
        self.best_char_idx
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedInput {
    pub lines: Vec<String>,
    pub col: usize,
    pub row: usize,
}

struct WrapBuilder {
    lines: Vec<String>,
    row: usize,
    col: usize,
    cursor_pos: Option<(usize, usize)>,
    target_width: usize,
    cursor: usize,
}

impl WrapBuilder {
    fn new(cursor: usize, width: usize) -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            cursor_pos: None,
            target_width: width.max(1),
            cursor,
        }
    }

    fn push_newline(&mut self, idx: usize) {
        if idx == self.cursor {
            self.cursor_pos = Some((self.col, self.row));
        }
        self.lines.push(String::new());
        self.row += 1;
        self.col = 0;
    }

    fn push_char(&mut self, idx: usize, c: char) {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if self.col > 0 && self.col + cw > self.target_width {
            self.lines.push(String::new());
            self.row += 1;
            self.col = 0;
        }
        if idx == self.cursor {
            self.cursor_pos = Some((self.col, self.row));
        }
        self.lines[self.row].push(c);
        self.col += cw;
    }

    fn finish(mut self, text_len: usize) -> WrappedInput {
        if self.cursor >= text_len {
            if self.col >= self.target_width {
                self.lines.push(String::new());
                self.row += 1;
                self.col = 0;
            }
            self.cursor_pos = Some((self.col, self.row));
        }
        let (col, row) = self.cursor_pos.unwrap_or((self.col, self.row));
        WrappedInput {
            lines: self.lines,
            col,
            row,
        }
    }
}

pub fn wrap_input_text(text: &str, cursor: usize, width: usize) -> WrappedInput {
    let mut builder = WrapBuilder::new(cursor, width);
    for (idx, c) in text.chars().enumerate() {
        if c == '\n' {
            builder.push_newline(idx);
        } else {
            builder.push_char(idx, c);
        }
    }
    builder.finish(text.chars().count())
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

    #[cfg(test)]
    pub fn line_count(&self) -> usize {
        self.visual_line_count(usize::MAX)
    }

    #[cfg(test)]
    pub fn cursor_coords(&self) -> (usize, usize) {
        self.visual_coords(usize::MAX)
    }

    pub fn visual_line_count(&self, width: usize) -> usize {
        wrap_input_text(&self.value, self.cursor, width).lines.len()
    }

    pub fn visual_coords(&self, width: usize) -> (usize, usize) {
        let wrapped = wrap_input_text(&self.value, self.cursor, width);
        (wrapped.col, wrapped.row)
    }

    pub fn move_visual_up(&mut self, width: usize) -> bool {
        let (cur_col, cur_row) = self.visual_coords(width);
        if cur_row == 0 {
            return false;
        }
        self.cursor = self.char_idx_for_visual_coords(VisualCoords {
            col: cur_col,
            row: cur_row - 1,
            width,
        });
        true
    }

    pub fn move_visual_down(&mut self, width: usize) -> bool {
        let (cur_col, cur_row) = self.visual_coords(width);
        let total_lines = self.visual_line_count(width);
        if cur_row + 1 >= total_lines {
            return false;
        }
        self.cursor = self.char_idx_for_visual_coords(VisualCoords {
            col: cur_col,
            row: cur_row + 1,
            width,
        });
        true
    }

    #[cfg(test)]
    pub fn move_up(&mut self) -> bool {
        self.move_visual_up(usize::MAX)
    }

    #[cfg(test)]
    pub fn move_down(&mut self) -> bool {
        self.move_visual_down(usize::MAX)
    }

    fn char_idx_for_visual_coords(&self, target: VisualCoords) -> usize {
        let mut finder = CoordFinder::new(target);
        for (idx, c) in self.value.chars().enumerate() {
            if finder.step_char(idx, c) {
                return finder.best_char_idx;
            }
        }
        finder.finish(self.value.chars().count())
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

    fn prev_word_boundary(&self) -> usize {
        let s = &self.value[..self.byte_idx()];
        s.trim_end().rfind(char::is_whitespace).map_or(0, |x| x + 1)
    }

    pub fn delete_prev_word(&mut self) {
        let b = self.byte_idx();
        let i = self.prev_word_boundary();
        self.value.drain(i..b);
        self.cursor = self.value[..i].chars().count();
    }

    pub fn move_prev_word(&mut self) {
        let i = self.prev_word_boundary();
        self.cursor = self.value[..i].chars().count();
    }

    pub fn move_next_word(&mut self) {
        let s = &self.value[self.byte_idx()..];
        let trimmed = s.trim_start();
        let skip = s.len() - trimmed.len();
        let w = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        self.cursor += s[..skip + w].chars().count();
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

    pub fn set_value(&mut self, val: String) {
        self.cursor = val.chars().count();
        self.value = val;
    }

    pub fn take_value(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_wrap_input_text_non_word_boundary() {
        // AC-001 & AC-002: wraps at exact column boundary without breaking at words
        let wrapped = wrap_input_text("1234567890abcdef", 12, 10);
        assert_eq!(wrapped.lines, vec!["1234567890", "abcdef"]);
        assert_eq!(wrapped.col, 2);
        assert_eq!(wrapped.row, 1);

        // Empty string
        let empty = wrap_input_text("", 0, 10);
        assert_eq!(empty.lines, vec![""]);
        assert_eq!(empty.col, 0);
        assert_eq!(empty.row, 0);

        // Cursor at boundary of exactly full line
        let full = wrap_input_text("1234567890", 10, 10);
        assert_eq!(full.lines, vec!["1234567890", ""]);
        assert_eq!(full.col, 0);
        assert_eq!(full.row, 1);

        // Cursor in the middle of full line
        let mid = wrap_input_text("1234567890", 5, 10);
        assert_eq!(mid.lines, vec!["1234567890"]);
        assert_eq!(mid.col, 5);
        assert_eq!(mid.row, 0);

        // Explicit newlines combined with wrapping
        let mixed = wrap_input_text("12345\n1234567890abcdef", 18, 10);
        assert_eq!(mixed.lines, vec!["12345", "1234567890", "abcdef"]);
        assert_eq!(mixed.col, 2);
        assert_eq!(mixed.row, 2);

        // Wide characters (CJK)
        let wide = wrap_input_text("1234界", 5, 5);
        assert_eq!(wide.lines, vec!["1234", "界"]);
        assert_eq!(wide.col, 2);
        assert_eq!(wide.row, 1);
    }

    #[test]
    fn test_input_cursor_coords() {
        let mut input = InputState::default();
        assert_eq!(input.line_count(), 1);
        assert_eq!(input.cursor_coords(), (0, 0));

        input.set_value("first\nsecond\nthird".into());
        assert_eq!(input.cursor(), 18);
        assert_eq!(input.line_count(), 3);
        assert_eq!(input.cursor_coords(), (5, 2));

        input.move_start();
        assert_eq!(input.cursor(), 0);
        assert_eq!(input.cursor_coords(), (0, 0));

        for _ in 0..5 {
            input.move_right();
        }
        assert_eq!(input.cursor_coords(), (5, 0));

        input.move_right();
        assert_eq!(input.cursor_coords(), (0, 1));
    }

    #[test]
    fn test_input_visual_vertical_movement_wrapped() {
        let mut input = InputState::default();
        // Width 10: "0123456789" (row 0), "abcdef" (row 1)
        input.set_value("0123456789abcdef".into());
        // Cursor starts at end: char 16, row 1, col 6
        assert_eq!(input.visual_coords(10), (6, 1));
        assert!(!input.move_visual_down(10));

        // Move up from row 1, col 6 -> moves to row 0, col 6 (char '6' at index 6)
        assert!(input.move_visual_up(10));
        assert_eq!(input.visual_coords(10), (6, 0));
        assert_eq!(input.cursor(), 6);

        // Cannot move up further
        assert!(!input.move_visual_up(10));

        // Move down back to row 1, col 6
        assert!(input.move_visual_down(10));
        assert_eq!(input.visual_coords(10), (6, 1));
        assert_eq!(input.cursor(), 16);

        // Position on row 1 at col 2 ('c', char 12)
        input.move_start();
        for _ in 0..12 {
            input.move_right();
        }
        assert_eq!(input.visual_coords(10), (2, 1));
        assert_eq!(input.cursor(), 12);

        // Move up to row 0, col 2 ('2', char 2)
        assert!(input.move_visual_up(10));
        assert_eq!(input.visual_coords(10), (2, 0));
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn test_input_vertical_movement() {
        let mut input = InputState::default();
        input.set_value("short\nlonger line\nabc".into());

        assert_eq!(input.cursor_coords(), (3, 2));
        assert!(!input.move_down());
        assert_eq!(input.cursor_coords(), (3, 2));

        assert!(input.move_up());
        assert_eq!(input.cursor_coords(), (3, 1));

        for _ in 0..8 {
            input.move_right();
        }
        assert_eq!(input.cursor_coords(), (11, 1));

        assert!(input.move_up());
        assert_eq!(input.cursor_coords(), (5, 0));
        assert!(!input.move_up());

        assert!(input.move_down());
        assert_eq!(input.cursor_coords(), (5, 1));

        assert!(input.move_down());
        assert_eq!(input.cursor_coords(), (3, 2));
    }
}
