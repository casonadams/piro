pub mod blocks;
pub mod feed;
pub mod footer;
pub mod input;
pub mod text;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::Paragraph,
};

use crate::app::App;
use feed::draw_feed;
use footer::draw_footer;
use input::draw_input;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let size = frame.area();

    // Layout:
    // <output area> (feed)
    // <empty space separator> (1 empty row)
    // <user input area> (3 rows: top line, text row, bottom line)
    // <footer area> (1 row, status badge + scroll notification)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Output feed
            Constraint::Length(1), // Empty space separator
            Constraint::Length(3), // User input area (top line, text row, bottom line)
            Constraint::Length(1), // Footer
        ])
        .split(size);

    let feed_area = chunks[0];
    let empty_sep_area = chunks[1];
    let input_area = chunks[2];
    let footer_area = chunks[3];

    draw_feed(frame, feed_area, app);
    draw_empty_separator(frame, empty_sep_area);
    draw_input(frame, input_area, app);
    draw_footer(frame, footer_area, app);
}

fn draw_empty_separator(frame: &mut Frame, area: Rect) {
    let p = Paragraph::new("");
    frame.render_widget(p, area);
}
