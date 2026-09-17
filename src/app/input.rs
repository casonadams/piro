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
    fn test_input_cursor_coords() {
        let mut input = InputState::default();
        assert_eq!(input.line_count(), 1);
        assert_eq!(input.cursor_coords(), (0, 0));

        input.set_value("first\nsecond\nthird".into());
        assert_eq!(input.line_count(), 3);
        assert_eq!(input.cursor_coords(), (5, 2));

        input.move_start();
        assert_eq!(input.cursor_coords(), (0, 0));

        for _ in 0..5 {
            input.move_right();
        }
        assert_eq!(input.cursor_coords(), (5, 0));

        input.move_right();
        assert_eq!(input.cursor_coords(), (0, 1));
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
