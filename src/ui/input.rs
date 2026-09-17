use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::InputState;

pub fn draw_input(frame: &mut Frame, area: Rect, input: &InputState) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let display_text = input.value();
    let p = Paragraph::new(display_text).block(block);
    frame.render_widget(p, area);

    let cursor_x = area.x + (input.cursor() as u16);
    let cursor_y = area.y + 1;
    if cursor_x < area.x + area.width && cursor_y < area.y + area.height {
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}
