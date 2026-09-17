use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use super::blocks::{render_command_output_block, render_user_input_block};
use crate::app::{App, FeedItem, Mode};

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

    // In Scroll Mode, highlight the cursor line and visual selection range
    if app.mode == Mode::Scroll {
        let sel = app
            .selection_anchor
            .map(|a| (a.min(app.cursor_line), a.max(app.cursor_line)));

        for (i, line) in visible.iter_mut().enumerate() {
            let abs_idx = start + i;
            let is_selected = sel.is_some_and(|(min, max)| abs_idx >= min && abs_idx <= max);
            let is_cursor = abs_idx == app.cursor_line;

            if is_selected {
                // Visual selection highlight (reversed / high-contrast)
                for span in line.spans.iter_mut() {
                    span.style = span.style.add_modifier(Modifier::REVERSED);
                }
            } else if is_cursor {
                // Cursor line highlight
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
}

pub fn get_selected_text(app: &App, width: usize) -> String {
    let mut all_lines = Vec::new();
    for item in &app.feed {
        match item {
            FeedItem::UserInput(cmd) => all_lines.extend(render_user_input_block(cmd, width)),
            FeedItem::CommandOutput(b) => all_lines.extend(render_command_output_block(b, width)),
        }
        all_lines.push(Line::from(""));
    }

    let (start_idx, end_idx) = match app.selection_anchor {
        Some(a) => (a.min(app.cursor_line), a.max(app.cursor_line)),
        None => (app.cursor_line, app.cursor_line),
    };

    let mut result = Vec::new();
    for (i, line) in all_lines.iter().enumerate() {
        if i >= start_idx && i <= end_idx {
            let raw: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let cleaned = clean_line_for_copy(&raw);
            if !cleaned.is_empty() {
                result.push(cleaned);
            }
        }
    }

    result.join("\n")
}

fn clean_line_for_copy(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('┌') || trimmed.starts_with('└') {
        return "".into();
    }
    let mut s = trimmed;
    if s.starts_with('│') {
        s = &s['│'.len_utf8()..];
    }
    if s.ends_with('│') {
        s = &s[..s.len() - '│'.len_utf8()];
    }
    s.trim().to_string()
}
