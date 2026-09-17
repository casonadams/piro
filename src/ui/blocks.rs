use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::app::{CommandOutputBlock, CommandStatus};
use crate::ui::text::{pad_spaces, str_width, wrap_ansi};

pub const TAIL_LINES: usize = 8;

pub struct FormattedLine {
    pub line: Line<'static>,
    pub is_continuation: bool,
}

#[derive(Clone, Copy)]
struct BorderCaps {
    pub left: char,
    pub fill: char,
    pub right: char,
}

const TOP_CAPS: BorderCaps = BorderCaps {
    left: '┌',
    fill: '─',
    right: '┐',
};
const BOTTOM_CAPS: BorderCaps = BorderCaps {
    left: '└',
    fill: '─',
    right: '┘',
};

fn box_border(caps: BorderCaps, width: usize, style: Style) -> Line<'static> {
    let inner = caps.fill.to_string().repeat(width.saturating_sub(2));
    Line::from(vec![Span::styled(
        format!("{}{}{}", caps.left, inner, caps.right),
        style,
    )])
}

fn box_titled_border(title: &str, width: usize, style: Style) -> Line<'static> {
    let t_len = str_width(title);
    if width < t_len + 3 {
        return box_border(TOP_CAPS, width, style);
    }
    let rem = width.saturating_sub(3 + t_len);
    let border_str = format!("┌─{}{}{}┐", title, "─".repeat(rem), "");
    Line::from(vec![Span::styled(border_str, style)])
}

fn box_line(spans: Vec<Span<'static>>, border_style: Style) -> Line<'static> {
    let mut line_spans = vec![Span::styled("│ ", border_style)];
    line_spans.extend(spans);
    line_spans.push(Span::styled(" │", border_style));
    Line::from(line_spans)
}

struct TextCard<'a> {
    title: &'a str,
    text: &'a str,
    color: Color,
    empty_hint: Option<&'a str>,
}

fn render_text_card(card: TextCard, width: usize) -> Vec<FormattedLine> {
    let style = Style::default().fg(card.color);
    let dim = Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM);
    let inner_w = width.saturating_sub(4);
    let top_border = if card.title.is_empty() {
        box_border(TOP_CAPS, width, style)
    } else {
        box_titled_border(card.title, width, style)
    };
    let mut lines = vec![FormattedLine {
        line: top_border,
        is_continuation: false,
    }];
    if card.text.trim().is_empty() {
        if let Some(hint) = card.empty_hint {
            lines.push(FormattedLine {
                line: box_line(vec![Span::styled(pad_spaces(hint, inner_w), dim)], style),
                is_continuation: false,
            });
        }
    } else {
        for wrapped in wrap_ansi(card.text, inner_w) {
            lines.push(FormattedLine {
                line: box_line(wrapped.spans, style),
                is_continuation: wrapped.is_continuation,
            });
        }
    }
    lines.push(FormattedLine {
        line: box_border(BOTTOM_CAPS, width, style),
        is_continuation: false,
    });
    lines
}

pub fn render_user_input_block(command: &str, width: usize) -> Vec<FormattedLine> {
    render_text_card(
        TextCard {
            title: "",
            text: command,
            color: Color::Cyan,
            empty_hint: None,
        },
        width,
    )
}

pub fn render_agent_thought_block(thought: &str, width: usize) -> Vec<FormattedLine> {
    render_text_card(
        TextCard {
            title: " Thought ",
            text: thought,
            color: Color::Magenta,
            empty_hint: Some("Thinking..."),
        },
        width,
    )
}

pub fn render_agent_response_block(response: &str, width: usize) -> Vec<FormattedLine> {
    render_text_card(
        TextCard {
            title: " Agent ",
            text: response,
            color: Color::Cyan,
            empty_hint: None,
        },
        width,
    )
}

struct CardLayout {
    width: usize,
    inner_w: usize,
    border: Style,
    dim: Style,
}

fn empty_box_line(inner_w: usize, border: Style) -> FormattedLine {
    FormattedLine {
        line: box_line(vec![Span::raw(" ".repeat(inner_w))], border),
        is_continuation: false,
    }
}

fn render_cmd_header(cmd: &str, title: Option<&str>, layout: &CardLayout) -> Vec<FormattedLine> {
    let top_border = match title {
        Some(t) => box_titled_border(t, layout.width, layout.border),
        None => box_border(TOP_CAPS, layout.width, layout.border),
    };
    let mut lines = vec![FormattedLine {
        line: top_border,
        is_continuation: false,
    }];
    let cmd_raw = format!("$ {}", cmd);
    for wrapped in wrap_ansi(&cmd_raw, layout.inner_w) {
        let mut spans = wrapped.spans;
        if let Some(s) = spans.first_mut() {
            s.style = s.style.add_modifier(Modifier::BOLD);
        }
        lines.push(FormattedLine {
            line: box_line(spans, layout.border),
            is_continuation: wrapped.is_continuation,
        });
    }
    lines.push(empty_box_line(layout.inner_w, layout.border));
    lines
}

fn apply_tail_collapse(body: Vec<FormattedLine>, expanded: bool, layout: &CardLayout) -> Vec<FormattedLine> {
    if body.len() <= TAIL_LINES {
        return body;
    }
    let mut out = Vec::new();
    if !expanded {
        let hidden = body.len() - TAIL_LINES;
        let hint = format!("... {} lines hidden (Ctrl+O to expand) ...", hidden);
        out.push(FormattedLine {
            line: box_line(
                vec![Span::styled(pad_spaces(&hint, layout.inner_w), layout.dim)],
                layout.border,
            ),
            is_continuation: false,
        });
        out.extend(body.into_iter().skip(hidden));
    } else {
        out.push(FormattedLine {
            line: box_line(
                vec![Span::styled(
                    pad_spaces("(Ctrl+O to collapse)", layout.inner_w),
                    layout.dim,
                )],
                layout.border,
            ),
            is_continuation: false,
        });
        out.extend(body);
    }
    out
}

fn render_output_body(block: &CommandOutputBlock, layout: &CardLayout) -> Vec<FormattedLine> {
    if block.lines.is_empty() {
        return vec![FormattedLine {
            line: box_line(
                vec![Span::styled(pad_spaces("(no output)", layout.inner_w), layout.dim)],
                layout.border,
            ),
            is_continuation: false,
        }];
    }
    let body = block
        .lines
        .iter()
        .flat_map(|l| wrap_ansi(l, layout.inner_w))
        .map(|w| FormattedLine {
            line: box_line(w.spans, layout.border),
            is_continuation: w.is_continuation,
        })
        .collect();
    apply_tail_collapse(body, block.expanded, layout)
}

fn format_status_line(status: &CommandStatus, inner_w: usize, border: Style) -> Option<FormattedLine> {
    let (msg, color) = match status {
        CommandStatus::Failed(code) => (format!("Command exited with code {}", code), Color::Red),
        CommandStatus::Cancelled => ("Command cancelled".to_string(), Color::Yellow),
        _ => return None,
    };
    Some(FormattedLine {
        line: box_line(
            vec![Span::styled(
                pad_spaces(&msg, inner_w),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )],
            border,
        ),
        is_continuation: false,
    })
}

fn render_block_footer(block: &CommandOutputBlock, layout: &CardLayout) -> Vec<FormattedLine> {
    let mut lines = vec![empty_box_line(layout.inner_w, layout.border)];
    if let Some(st_line) = format_status_line(&block.status, layout.inner_w, layout.border) {
        lines.push(st_line);
        lines.push(empty_box_line(layout.inner_w, layout.border));
    }
    let dur = block.duration.unwrap_or_else(|| block.started_at.elapsed());
    let status_str = match block.status {
        CommandStatus::Running => format!("Running {:.1}s", dur.as_secs_f64()),
        _ => format!("Took {:.1}s", dur.as_secs_f64()),
    };
    lines.push(FormattedLine {
        line: box_line(
            vec![Span::styled(pad_spaces(&status_str, layout.inner_w), layout.dim)],
            layout.border,
        ),
        is_continuation: false,
    });
    lines.push(FormattedLine {
        line: box_border(BOTTOM_CAPS, layout.width, layout.border),
        is_continuation: false,
    });
    lines
}

pub fn render_command_output_block(block: &CommandOutputBlock, width: usize) -> Vec<FormattedLine> {
    let color = match block.status {
        CommandStatus::Running => Color::Yellow,
        CommandStatus::Success(_) => Color::Green,
        CommandStatus::Failed(_) => Color::Red,
        CommandStatus::Cancelled => Color::DarkGray,
    };
    let layout = CardLayout {
        width,
        inner_w: width.saturating_sub(4),
        border: Style::default().fg(color),
        dim: Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    };

    let title = block.tool.as_deref().map(|t| format!(" Tool: {} ", t));
    let mut lines = render_cmd_header(&block.command, title.as_deref(), &layout);
    lines.extend(render_output_body(block, &layout));
    lines.extend(render_block_footer(block, &layout));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_render_thought_and_response_blocks() {
        let width = 40;
        let thought_lines = render_agent_thought_block("Considering the next action...", width);
        assert!(thought_lines.len() >= 3);
        let top_str = &thought_lines[0].line.spans[0].content;
        assert!(top_str.contains("Thought"));
        assert_eq!(str_width(top_str), width);

        let response_lines = render_agent_response_block("Here is the final answer.", width);
        assert!(response_lines.len() >= 3);
        let top_resp = &response_lines[0].line.spans[0].content;
        assert!(top_resp.contains("Agent"));
        assert_eq!(str_width(top_resp), width);
    }

    #[test]
    fn test_render_tool_command_output_block() {
        let width = 50;
        let block = CommandOutputBlock {
            command: "echo test".into(),
            tool: Some("bash".into()),
            status: CommandStatus::Success(0),
            lines: vec!["output 1".into(), "output 2".into()],
            started_at: Instant::now(),
            duration: Some(std::time::Duration::from_millis(150)),
            expanded: false,
        };
        let lines = render_command_output_block(&block, width);
        let top_str = &lines[0].line.spans[0].content;
        assert!(top_str.contains("Tool: bash"));
        assert_eq!(str_width(top_str), width);
    }

    #[test]
    fn test_render_user_input_multiline_block() {
        let width = 40;
        let lines = render_user_input_block("first line\nsecond line", width);
        // Top border + 2 content lines + bottom border = 4 lines
        assert_eq!(lines.len(), 4);
        assert!(!lines[1].is_continuation);
        assert!(!lines[2].is_continuation);

        let row1: String = lines[1].line.spans.iter().map(|s| s.content.as_ref()).collect();
        let row2: String = lines[2].line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(row1.contains("first line"));
        assert!(row2.contains("second line"));
        assert!(!row1.contains("second line"));
    }
}
