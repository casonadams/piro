use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Position, Rect},
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use super::blocks::{
    FormattedLine, render_agent_response_block, render_agent_thought_block, render_command_output_block,
    render_user_input_block,
};
use crate::app::{App, FeedItem, Mode, SelectionType};

pub fn format_feed_item(item: &FeedItem, width: usize) -> Vec<FormattedLine> {
    match item {
        FeedItem::UserInput(cmd) => render_user_input_block(cmd, width),
        FeedItem::CommandOutput(b) => render_command_output_block(b, width),
        FeedItem::AgentThought(t) => render_agent_thought_block(t, width),
        FeedItem::AgentResponse(r) => render_agent_response_block(r, width),
    }
}

pub fn format_entire_feed(feed: &[FeedItem], width: usize) -> Vec<FormattedLine> {
    let mut formatted = Vec::new();
    for item in feed {
        formatted.extend(format_feed_item(item, width));
        formatted.push(FormattedLine {
            line: Line::from(""),
            is_continuation: false,
        });
    }
    formatted
}

struct ScrollView<'a> {
    app: &'a App,
    start: usize,
    pad: usize,
    area: Rect,
}

fn render_scroll_cursor(frame: &mut Frame, view: &ScrollView, end: usize) {
    if view.app.mode != Mode::Scroll {
        return;
    }
    if view.app.normalized_selection().is_some() {
        apply_doc_selection(frame.buffer_mut(), view);
    }
    if view.app.cursor.line >= view.start && view.app.cursor.line < end {
        let row = view.area.y + (view.pad + (view.app.cursor.line - view.start)) as u16;
        let col = view.area.x + view.app.cursor.col.min(view.area.width.saturating_sub(1));
        frame.set_cursor_position(Position::new(col, row));
    }
}

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

    let formatted = format_entire_feed(&app.feed, width);
    app.total_feed_lines = formatted.len();
    let (start, end) = app.scroll.visible_range(formatted.len(), view_h);
    let visible: Vec<Line<'static>> = formatted[start..end].iter().map(|fl| fl.line.clone()).collect();

    let count = visible.len();
    let pad = if count < view_h && app.scroll.auto_scroll {
        view_h - count
    } else {
        0
    };

    let mut padded = vec![Line::from(""); pad];
    padded.extend(visible);
    frame.render_widget(Paragraph::new(padded), area);

    let view = ScrollView { app, start, pad, area };
    render_scroll_cursor(frame, &view, end);
}

fn apply_doc_selection(buf: &mut Buffer, view: &ScrollView) {
    let Some((sel_start, sel_end)) = view.app.normalized_selection() else {
        return;
    };
    let right = view.area.right().saturating_sub(1);

    for (row_idx, line_num) in (view.start..).enumerate() {
        let screen_row = view.area.y + (view.pad + row_idx) as u16;
        if screen_row >= view.area.bottom() {
            break;
        }

        if line_num >= sel_start.line && line_num <= sel_end.line {
            let (col_start, col_end) = match view.app.selection_type {
                SelectionType::Line => (view.area.x, right),
                SelectionType::Char => {
                    if sel_start.line == sel_end.line {
                        (sel_start.col.min(sel_end.col), sel_start.col.max(sel_end.col))
                    } else if line_num == sel_start.line {
                        (sel_start.col, right)
                    } else if line_num == sel_end.line {
                        (view.area.x, sel_end.col)
                    } else {
                        (view.area.x, right)
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

fn slice_char_selection(cleaned: &str, sel_start: &crate::app::DocPos, sel_end: &crate::app::DocPos) -> String {
    let start_col = (sel_start.col.min(sel_end.col) as usize).saturating_sub(2);
    let end_col = (sel_start.col.max(sel_end.col) as usize).saturating_sub(2);
    let chars: Vec<char> = cleaned.chars().collect();
    if start_col < chars.len() {
        chars[start_col..end_col.min(chars.len())].iter().collect()
    } else {
        cleaned.to_string()
    }
}

pub fn extract_selected_text(app: &App, width: usize) -> String {
    let all_formatted = format_entire_feed(&app.feed, width);
    let (sel_start, sel_end) = app.normalized_selection().unwrap_or((app.cursor, app.cursor));
    let start_idx = sel_start.line.min(all_formatted.len());
    let end_limit = (sel_end.line + 1).min(all_formatted.len());

    let mut out = String::new();
    if start_idx >= end_limit {
        return out;
    }

    for (offset, item) in all_formatted[start_idx..end_limit].iter().enumerate() {
        let line_idx = start_idx + offset;
        let raw: String = item.line.spans.iter().map(|s| s.content.as_ref()).collect();
        let cleaned = clean_scraped_line(&raw);
        if cleaned.is_empty() {
            continue;
        }

        let snippet = if app.selection_type == SelectionType::Char && sel_start.line == sel_end.line {
            slice_char_selection(&cleaned, &sel_start, &sel_end)
        } else {
            cleaned
        };

        if snippet.is_empty() {
            continue;
        }

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
    out
}

fn clean_scraped_line(raw: &str) -> String {
    let s = raw.trim();
    if (s.starts_with('┌') && s.ends_with('┐')) || (s.starts_with('└') && s.ends_with('┘')) {
        return String::new();
    }
    let s = s.strip_prefix("│ ").unwrap_or(s);
    let s = s.strip_suffix(" │").unwrap_or(s).trim();
    if s.chars().all(|c| matches!(c, '─' | '┌' | '┐' | '└' | '┘' | ' ')) {
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
        let mut app = App::default();
        let long_line =
            "This is a very long line that will definitely exceed forty characters and wrap without soft line breaks";
        app.feed.push(FeedItem::CommandOutput(CommandOutputBlock {
            command: "echo test".into(),
            tool: None,
            status: CommandStatus::Success(0),
            lines: vec![long_line.into()],
            started_at: Instant::now(),
            duration: None,
            expanded: true,
        }));

        let width = 30;
        let mut formatted = Vec::new();
        for item in &app.feed {
            if let FeedItem::CommandOutput(b) = item {
                formatted.extend(render_command_output_block(b, width));
            }
        }

        app.total_feed_lines = formatted.len() + 1;
        app.enter_scroll_mode();
        app.selection_type = SelectionType::Line;
        app.selection_anchor = Some(DocPos { line: 0, col: 0 });
        app.cursor.line = formatted.len();

        let extracted = extract_selected_text(&app, width);
        assert!(
            extracted.contains(long_line),
            "extracted text must contain unbroken long line: {extracted:?}"
        );
    }

    #[test]
    fn test_extract_thought_and_response_text() {
        let mut app = App::default();
        app.feed.push(FeedItem::AgentThought("Reasoning about solution".into()));
        app.feed.push(FeedItem::CommandOutput(CommandOutputBlock {
            command: "echo test".into(),
            tool: Some("bash".into()),
            status: CommandStatus::Success(0),
            lines: vec!["tool result line".into()],
            started_at: Instant::now(),
            duration: Some(std::time::Duration::from_millis(10)),
            expanded: false,
        }));
        app.feed.push(FeedItem::AgentResponse("Final answer here".into()));

        let width = 40;
        let formatted = format_entire_feed(&app.feed, width);

        app.total_feed_lines = formatted.len();
        app.enter_scroll_mode();
        app.selection_type = SelectionType::Line;
        app.selection_anchor = Some(DocPos { line: 0, col: 0 });
        app.cursor.line = formatted.len();

        let extracted = extract_selected_text(&app, width);
        assert!(extracted.contains("Reasoning about solution"));
        assert!(extracted.contains("tool result line"));
        assert!(extracted.contains("Final answer here"));
        assert!(!extracted.contains("┌─"));
        assert!(!extracted.contains("└─"));
    }
}
