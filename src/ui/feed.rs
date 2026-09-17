use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
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

    // In Keyboard Scroll Mode without mouse selection, highlight cursor and line selection
    if app.mode == Mode::Scroll && app.mouse_selection.is_none() {
        let sel = app
            .selection_anchor
            .map(|a| (a.min(app.cursor_line), a.max(app.cursor_line)));

        for (i, line) in visible.iter_mut().enumerate() {
            let abs_idx = start + i;
            let is_selected = sel.is_some_and(|(min, max)| abs_idx >= min && abs_idx <= max);
            let is_cursor = abs_idx == app.cursor_line;

            if is_selected {
                for span in line.spans.iter_mut() {
                    span.style = span.style.add_modifier(Modifier::REVERSED);
                }
            } else if is_cursor {
                for span in line.spans.iter_mut() {
                    span.style = span.style.bg(Color::Rgb(35, 45, 60)).add_modifier(Modifier::BOLD);
                }
            }
        }
    }

    // Bottom-anchor content if it fits within the viewport height and auto-scrolling
    if visible.len() < view_h && app.scroll.auto_scroll {
        let pad = view_h - visible.len();
        let mut padded = vec![Line::from(""); pad];
        padded.extend(visible);
        visible = padded;
    }

    frame.render_widget(Paragraph::new(visible), area);

    // Apply mouse drag highlight directly on frame buffer cells (character precision)
    if let Some(ref sel) = app.mouse_selection {
        apply_mouse_highlight(frame.buffer_mut(), sel, area);
    }
}

pub fn apply_mouse_highlight(buf: &mut Buffer, sel: &TextSelection, area: Rect) {
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

pub fn extract_mouse_selection_from_buf(buf: &Buffer, sel: &TextSelection, area: Rect) -> String {
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

pub fn get_keyboard_selected_text(app: &App, width: usize) -> String {
    let mut all_lines = Vec::new();
    for item in &app.feed {
        match item {
            FeedItem::UserInput(cmd) => all_lines.extend(render_user_input_block(cmd, width)),
            FeedItem::CommandOutput(b) => all_lines.extend(render_command_output_block(b, width)),
        }
        lines_push_spacer(&mut all_lines);
    }

    let (start_idx, end_idx) = match app.selection_anchor {
        Some(a) => (a.min(app.cursor_line), a.max(app.cursor_line)),
        None => (app.cursor_line, app.cursor_line),
    };

    let mut result = Vec::new();
    for (i, line) in all_lines.iter().enumerate() {
        if i >= start_idx && i <= end_idx {
            let raw: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let cleaned = clean_scraped_line(&raw);
            if !cleaned.is_empty() {
                result.push(cleaned);
            }
        }
    }

    result.join("\n")
}

fn lines_push_spacer(lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from(""));
}
