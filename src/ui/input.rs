use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, Mode};

pub fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let display_text = app.input.value();
    let p = Paragraph::new(display_text).block(block);
    frame.render_widget(p, area);

    // Only position and show cursor when in Input mode
    if app.mode == Mode::Input {
        let (col, row) = app.input.cursor_coords();
        let cursor_x = area.x + (col as u16);
        let cursor_y = area.y + 1 + (row as u16);
        if cursor_x < area.x + area.width && cursor_y < area.y + area.height.saturating_sub(1) {
            frame.set_cursor_position(Position::new(cursor_x, cursor_y));
        }
    }
}
