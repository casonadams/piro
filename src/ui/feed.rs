use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use super::blocks::{render_command_output_block, render_user_input_block};
use crate::app::{App, FeedItem, Mode, TextSelection};

pub fn draw_feed(frame: &mut Frame, area: Rect, app: &mut App) {
    let view_h = area.height as usize;
    app.viewport_height = view_h;

    if app.feed.is_empty() {
        app.total_feed_lines = 0;
        let placeholder = Paragraph::new("No commands executed yet. Type a command below and press Enter.")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(placeholder, area);
        return;
    }

    let width = (area.width as usize).saturating_sub(1);
    if width < 10 {
        return;
    }

    let mut lines = Vec::new();
    for item in &app.feed {
        match item {
            FeedItem::UserInput(cmd) => lines.extend(render_user_input_block(cmd, width)),
            FeedItem::CommandOutput(b) => lines.extend(render_command_output_block(b, width)),
        }
        lines.push(Line::from(""));
    }

    app.total_feed_lines = lines.len();
    let (start, end) = app.scroll.visible_range(lines.len(), view_h);
    let mut visible: Vec<Line<'static>> = lines.into_iter().skip(start).take(end - start).collect();

    // Bottom-anchor content if it fits within the viewport height and auto-scrolling
    if visible.len() < view_h && app.scroll.auto_scroll {
        let pad = view_h - visible.len();
        let mut padded = vec![Line::from(""); pad];
        padded.extend(visible);
        visible = padded;
    }

    frame.render_widget(Paragraph::new(visible), area);

    // Apply visual selection highlight across cells in the buffer
    let max_col = area.width.saturating_sub(1);
    if let Some(sel) = app.current_selection(max_col) {
        apply_selection_highlight(frame.buffer_mut(), &sel, area);
    }

    // In Scroll Mode, place the hardware terminal cursor directly at the cursor position
    if app.mode == Mode::Scroll {
        let cur_x = area.x + app.cursor_pos.col.min(area.width.saturating_sub(1));
        let cur_y = area.y + app.cursor_pos.row.min(area.height.saturating_sub(1));
        frame.set_cursor_position(Position::new(cur_x, cur_y));
    }
}

pub fn apply_selection_highlight(buf: &mut Buffer, sel: &TextSelection, area: Rect) {
    let (start, end) = sel.normalized();
    let row_start = start.row.clamp(area.y, area.bottom().saturating_sub(1));
    let row_end = end.row.clamp(area.y, area.bottom().saturating_sub(1));

    for row in row_start..=row_end {
        let (col_start, col_end) = if start.row == end.row {
            (start.col.min(end.col), start.col.max(end.col))
        } else if row == start.row {
            (start.col, area.right().saturating_sub(1))
        } else if row == end.row {
            (area.x, end.col)
        } else {
            (area.x, area.right().saturating_sub(1))
        };

        for col in col_start..=col_end {
            if col < buf.area().right() && row < buf.area().bottom() {
                let cell = &mut buf[(col, row)];
                cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
            }
        }
    }
}

pub fn extract_selection_from_buf(buf: &Buffer, sel: &TextSelection, area: Rect) -> String {
    let (start, end) = sel.normalized();
    let row_start = start.row.clamp(area.y, area.bottom().saturating_sub(1));
    let row_end = end.row.clamp(area.y, area.bottom().saturating_sub(1));

    let mut lines = Vec::new();

    for row in row_start..=row_end {
        let (col_start, col_end) = if start.row == end.row {
            (start.col.min(end.col), start.col.max(end.col))
        } else if row == start.row {
            (start.col, area.right().saturating_sub(1))
        } else if row == end.row {
            (area.x, end.col)
        } else {
            (area.x, area.right().saturating_sub(1))
        };

        let mut row_chars = String::new();
        for col in col_start..=col_end {
            if col < buf.area().right() && row < buf.area().bottom() {
                let cell = &buf[(col, row)];
                row_chars.push_str(cell.symbol());
            }
        }

        let cleaned = clean_scraped_line(&row_chars);
        lines.push(cleaned);
    }

    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

fn clean_scraped_line(raw: &str) -> String {
    let mut s = raw.trim_end();
    if s.starts_with("│ ") {
        s = &s["│ ".len()..];
    } else if s.starts_with('│') {
        s = &s['│'.len_utf8()..];
    }
    if s.ends_with(" │") {
        s = &s[..s.len() - " │".len()];
    } else if s.ends_with('│') {
        s = &s[..s.len() - '│'.len_utf8()];
    }
    if s.chars()
        .all(|c| c == '─' || c == '┌' || c == '┐' || c == '└' || c == '┘' || c == ' ')
    {
        return String::new();
    }
    s.to_string()
}
