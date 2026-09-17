use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use super::blocks::{render_command_output_block, render_user_input_block};
use crate::app::{App, FeedItem, Mode, SelectionType};

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

    let count = visible.len();
    let pad = if count < view_h && app.scroll.auto_scroll {
        view_h - count
    } else {
        0
    };

    // Bottom-anchor content if it fits within the viewport height and auto-scrolling
    if pad > 0 {
        let mut padded = vec![Line::from(""); pad];
        padded.extend(visible);
        visible = padded;
    }

    frame.render_widget(Paragraph::new(visible), area);

    // In Scroll Mode, apply visual selection highlight directly to buffer cells
    if app.mode == Mode::Scroll {
        if app.normalized_selection().is_some() {
            apply_doc_selection(frame.buffer_mut(), app, start, pad, area);
        }

        // Place the hardware cursor at the active DocPos if it's currently on screen
        if app.cursor.line >= start && app.cursor.line < end {
            let screen_row = area.y + (pad + (app.cursor.line - start)) as u16;
            let screen_col = area.x + app.cursor.col.min(area.width.saturating_sub(1));
            frame.set_cursor_position(Position::new(screen_col, screen_row));
        }
    }
}

fn apply_doc_selection(buf: &mut Buffer, app: &App, start_line: usize, pad: usize, area: Rect) {
    let Some((sel_start, sel_end)) = app.normalized_selection() else {
        return;
    };
    let right = area.right().saturating_sub(1);

    for (row_idx, line_num) in (start_line..).enumerate() {
        let screen_row = area.y + (pad + row_idx) as u16;
        if screen_row >= area.bottom() {
            break;
        }

        if line_num >= sel_start.line && line_num <= sel_end.line {
            let (col_start, col_end) = match app.selection_type {
                SelectionType::Line => (area.x, right),
                SelectionType::Char => {
                    if sel_start.line == sel_end.line {
                        (sel_start.col.min(sel_end.col), sel_start.col.max(sel_end.col))
                    } else if line_num == sel_start.line {
                        (sel_start.col, right)
                    } else if line_num == sel_end.line {
                        (area.x, sel_end.col)
                    } else {
                        (area.x, right)
                    }
                }
            };

            for col in col_start..=col_end {
                if col < buf.area().right() && screen_row < buf.area().bottom() {
                    let cell = &mut buf[(col, screen_row)];
                    cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
                }
            }
        }
    }
}

pub fn extract_selected_text(app: &App, width: usize) -> String {
    let mut all_lines = Vec::new();
    for item in &app.feed {
        match item {
            FeedItem::UserInput(cmd) => lines_extend(&mut all_lines, render_user_input_block(cmd, width)),
            FeedItem::CommandOutput(b) => lines_extend(&mut all_lines, render_command_output_block(b, width)),
        }
        all_lines.push(Line::from(""));
    }

    let (sel_start, sel_end) = match app.normalized_selection() {
        Some((s, e)) => (s, e),
        None => (app.cursor, app.cursor),
    };

    let mut result = Vec::new();
    let start_idx = sel_start.line.min(all_lines.len());
    let end_limit = (sel_end.line + 1).min(all_lines.len());

    if start_idx < end_limit {
        for line in &all_lines[start_idx..end_limit] {
            let raw: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let cleaned = clean_scraped_line(&raw);

            if cleaned.is_empty() {
                continue;
            }

            let snippet = if app.selection_type == SelectionType::Char && sel_start.line == sel_end.line {
                let start_col = (sel_start.col.min(sel_end.col) as usize).saturating_sub(2);
                let end_col = (sel_start.col.max(sel_end.col) as usize).saturating_sub(2);
                let chars: Vec<char> = cleaned.chars().collect();
                if start_col < chars.len() {
                    let end = end_col.min(chars.len());
                    chars[start_col..end].iter().collect()
                } else {
                    cleaned
                }
            } else {
                cleaned
            };

            if !snippet.is_empty() {
                result.push(snippet);
            }
        }
    }

    result.join("\n")
}

fn lines_extend(target: &mut Vec<Line<'static>>, source: Vec<Line<'static>>) {
    target.extend(source);
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
