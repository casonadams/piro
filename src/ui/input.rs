use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::input::{WrappedInput, wrap_input_text};
use crate::app::{App, Mode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputWindow {
    pub lines: Vec<String>,
    pub cursor_col: usize,
    pub cursor_row: usize,
}

pub fn window_input_lines(wrapped: WrappedInput, max_lines: usize) -> InputWindow {
    let total = wrapped.lines.len();
    let cap = max_lines.max(1);
    if total <= cap {
        return InputWindow {
            lines: wrapped.lines,
            cursor_col: wrapped.col,
            cursor_row: wrapped.row,
        };
    }
    let half = cap / 2;
    let ideal_start = wrapped.row.saturating_sub(half);
    let start = ideal_start.min(total.saturating_sub(cap));
    let windowed = wrapped.lines.into_iter().skip(start).take(cap).collect();
    InputWindow {
        lines: windowed,
        cursor_col: wrapped.col,
        cursor_row: wrapped.row.saturating_sub(start),
    }
}

pub fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let max_lines = area.height.saturating_sub(2) as usize;
    let wrapped = wrap_input_text(app.input.value(), app.input.cursor(), area.width as usize);
    let window = window_input_lines(wrapped, max_lines);

    let display_text = window.lines.join("\n");
    let p = Paragraph::new(display_text).block(block);
    frame.render_widget(p, area);

    // Only position and show cursor when in Input mode
    if app.mode == Mode::Input {
        let cursor_x = area.x + (window.cursor_col as u16);
        let cursor_y = area.y + 1 + (window.cursor_row as u16);
        if cursor_x < area.x + area.width && cursor_y < area.y + area.height.saturating_sub(1) {
            frame.set_cursor_position(Position::new(cursor_x, cursor_y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_input_lines_under_capacity() {
        let wrapped = WrappedInput {
            lines: vec!["line 1".into(), "line 2".into()],
            col: 3,
            row: 1,
        };
        let win = window_input_lines(wrapped, 5);
        assert_eq!(win.lines, vec!["line 1", "line 2"]);
        assert_eq!(win.cursor_col, 3);
        assert_eq!(win.cursor_row, 1);
    }

    #[test]
    fn test_window_input_lines_over_capacity() {
        let lines: Vec<String> = (0..15).map(|i| format!("line {}", i)).collect();
        // Cursor at line 14 (last line)
        let wrapped = WrappedInput { lines, col: 2, row: 14 };
        let win = window_input_lines(wrapped, 10);
        assert_eq!(win.lines.len(), 10);
        assert_eq!(win.lines[0], "line 5");
        assert_eq!(win.lines[9], "line 14");
        assert_eq!(win.cursor_col, 2);
        assert_eq!(win.cursor_row, 9);
    }

    #[test]
    fn test_window_input_lines_near_top() {
        let lines: Vec<String> = (0..15).map(|i| format!("line {}", i)).collect();
        // Cursor at line 2 (near top)
        let wrapped = WrappedInput { lines, col: 1, row: 2 };
        let win = window_input_lines(wrapped, 10);
        assert_eq!(win.lines.len(), 10);
        assert_eq!(win.lines[0], "line 0");
        assert_eq!(win.cursor_col, 1);
        assert_eq!(win.cursor_row, 2);
    }
}
