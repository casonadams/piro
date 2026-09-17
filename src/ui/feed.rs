use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Position, Rect},
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use super::blocks::{FormattedLine, render_command_output_block, render_user_input_block};
use crate::app::{App, FeedItem, Mode, SelectionType};

pub fn draw_feed(frame: &mut Frame, area: Rect, app: &mut App) {
    let view_h = area.height as usize;
    app.viewport_height = view_h;

    if app.feed.is_empty() {
        app.total_feed_lines = 0;
        let placeholder = Paragraph::new("No commands executed yet. Type a command below and press Enter.")
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(placeholder, area);
        return;
    }

    let width = area.width as usize;
    if width < 10 {
        return;
    }

    let mut formatted_lines: Vec<FormattedLine> = Vec::new();
    for item in &app.feed {
        match item {
            FeedItem::UserInput(cmd) => formatted_lines.extend(render_user_input_block(cmd, width)),
            FeedItem::CommandOutput(b) => formatted_lines.extend(render_command_output_block(b, width)),
        }
        formatted_lines.push(FormattedLine {
            line: Line::from(""),
            is_continuation: false,
        });
    }

    app.total_feed_lines = formatted_lines.len();
    let (start, end) = app.scroll.visible_range(formatted_lines.len(), view_h);
    let mut visible: Vec<Line<'static>> = formatted_lines
        .into_iter()
        .skip(start)
        .take(end - start)
        .map(|fl| fl.line)
        .collect();

    let count = visible.len();
    let pad = if count < view_h && app.scroll.auto_scroll {
        view_h - count
    } else {
        0
    };

    if pad > 0 {
        let mut padded = vec![Line::from(""); pad];
        padded.extend(visible);
        visible = padded;
    }

    frame.render_widget(Paragraph::new(visible), area);

    if app.mode == Mode::Scroll {
        if app.normalized_selection().is_some() {
            apply_doc_selection(frame.buffer_mut(), app, start, pad, area);
        }

        if app.cursor.line >= start && app.cursor.line < end {
            let row = area.y + (pad + (app.cursor.line - start)) as u16;
            let col = area.x + app.cursor.col.min(area.width.saturating_sub(1));
            frame.set_cursor_position(Position::new(col, row));
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
                    buf[(col, screen_row)].set_style(Modifier::REVERSED);
                }
            }
        }
    }
}

pub fn extract_selected_text(app: &App, width: usize) -> String {
    let mut all_formatted = Vec::new();
    for item in &app.feed {
        match item {
            FeedItem::UserInput(cmd) => all_formatted.extend(render_user_input_block(cmd, width)),
            FeedItem::CommandOutput(b) => all_formatted.extend(render_command_output_block(b, width)),
        }
        all_formatted.push(FormattedLine {
            line: Line::from(""),
            is_continuation: false,
        });
    }

    let (sel_start, sel_end) = app.normalized_selection().unwrap_or((app.cursor, app.cursor));
    let start_idx = sel_start.line.min(all_formatted.len());
    let end_limit = (sel_end.line + 1).min(all_formatted.len());

    let mut out = String::new();
    if start_idx < end_limit {
        for (offset, item) in all_formatted[start_idx..end_limit].iter().enumerate() {
            let line_idx = start_idx + offset;
            let raw: String = item.line.spans.iter().map(|s| s.content.as_ref()).collect();
            let cleaned = clean_scraped_line(&raw);
            if cleaned.is_empty() {
                continue;
            }

            // Apply column bounds for single-line character selection
            let snippet = if app.selection_type == SelectionType::Char && sel_start.line == sel_end.line {
                let start_col = (sel_start.col.min(sel_end.col) as usize).saturating_sub(2);
                let end_col = (sel_start.col.max(sel_end.col) as usize).saturating_sub(2);
                let chars: Vec<char> = cleaned.chars().collect();
                if start_col < chars.len() {
                    chars[start_col..end_col.min(chars.len())].iter().collect()
                } else {
                    cleaned
                }
            } else {
                cleaned
            };

            if snippet.is_empty() {
                continue;
            }

            // If this is a soft-wrapped continuation of the same logical line, unwrap without a hard newline!
            if item.is_continuation && line_idx > start_idx {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&snippet);
            } else {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&snippet);
            }
        }
    }

    out
}

fn clean_scraped_line(raw: &str) -> String {
    let s = raw.trim_end();
    let s = s.strip_prefix("│ ").unwrap_or(s);
    let s = s.strip_suffix(" │").unwrap_or(s);
    let s = s.trim();
    if s.chars()
        .all(|c| c == '─' || c == '┌' || c == '┐' || c == '└' || c == '┘' || c == ' ')
    {
        String::new()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{CommandOutputBlock, CommandStatus, DocPos};
    use std::time::Instant;

    #[test]
    fn test_extract_wrapped_line_without_soft_breaks() {
        let mut app = App::new();
        let long_line = "This is a very long line that will definitely exceed the forty character limit and should wrap cleanly without soft line breaks";
        app.feed.push(FeedItem::CommandOutput(CommandOutputBlock {
            command: "echo test".into(),
            status: CommandStatus::Success(0),
            lines: vec![long_line.into()],
            started_at: Instant::now(),
            duration: None,
        }));

        // Total width 30: long_line wraps to ~4 visual lines
        let width = 30;
        let mut formatted = Vec::new();
        for item in &app.feed {
            if let FeedItem::CommandOutput(b) = item {
                formatted.extend(render_command_output_block(b, width));
            }
        }

        // Verify it wrapped to multiple visual lines
        let continuation_count = formatted.iter().filter(|f| f.is_continuation).count();
        assert!(continuation_count > 0, "must wrap to multiple lines at width 30");

        // Select all lines of the feed
        app.total_feed_lines = formatted.len() + 1;
        app.enter_scroll_mode();
        app.selection_type = SelectionType::Line;
        app.selection_anchor = Some(DocPos { line: 0, col: 0 });
        app.cursor.line = formatted.len();

        let extracted = extract_selected_text(&app, width);

        // Verify the extracted string does NOT have soft-wrap breaks inside the long line
        assert!(
            extracted.contains(long_line),
            "extracted text must contain unbroken long line: {extracted:?}"
        );
    }
}
