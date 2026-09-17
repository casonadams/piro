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

fn box_border(left: char, fill: char, right: char, width: usize, style: Style) -> Line<'static> {
    let inner = fill.to_string().repeat(width.saturating_sub(2));
    Line::from(vec![Span::styled(format!("{}{}{}", left, inner, right), style)])
}

fn box_titled_border(left: char, title: &str, fill: char, right: char, width: usize, style: Style) -> Line<'static> {
    let t_len = str_width(title);
    if width < t_len + 3 {
        return box_border(left, fill, right, width, style);
    }
    let rem = width.saturating_sub(3 + t_len);
    let border_str = format!("{}{}{}{}{}", left, fill, title, fill.to_string().repeat(rem), right);
    Line::from(vec![Span::styled(border_str, style)])
}

fn box_line(spans: Vec<Span<'static>>, border_style: Style) -> Line<'static> {
    let mut line_spans = vec![Span::styled("│ ", border_style)];
    line_spans.extend(spans);
    line_spans.push(Span::styled(" │", border_style));
    Line::from(line_spans)
}

pub fn render_user_input_block(command: &str, width: usize) -> Vec<FormattedLine> {
    let style = Style::default().fg(Color::Cyan);
    let inner_w = width.saturating_sub(4);
    let mut lines = vec![FormattedLine {
        line: box_border('┌', '─', '┐', width, style),
        is_continuation: false,
    }];
    for (idx, wrapped) in wrap_ansi(command, inner_w).into_iter().enumerate() {
        lines.push(FormattedLine {
            line: box_line(wrapped, style),
            is_continuation: idx > 0,
        });
    }
    lines.push(FormattedLine {
        line: box_border('└', '─', '┘', width, style),
        is_continuation: false,
    });
    lines
}

pub fn render_command_block_with_title(
    title: Option<&str>,
    cmd: &str,
    block: &CommandOutputBlock,
    width: usize,
) -> Vec<FormattedLine> {
    let color = match block.status {
        CommandStatus::Running => Color::Yellow,
        CommandStatus::Success(_) => Color::Green,
        CommandStatus::Failed(_) => Color::Red,
        CommandStatus::Cancelled => Color::DarkGray,
    };
    let border = Style::default().fg(color);
    let dim = Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM);
    let inner_w = width.saturating_sub(4);
    let spacer = || FormattedLine {
        line: box_line(vec![Span::raw(" ".repeat(inner_w))], border),
        is_continuation: false,
    };

    let top_border = match title {
        Some(t) => box_titled_border('┌', t, '─', '┐', width, border),
        None => box_border('┌', '─', '┐', width, border),
    };

    let mut lines = vec![FormattedLine {
        line: top_border,
        is_continuation: false,
    }];

    // Command line
    let cmd_raw = format!("$ {}", cmd);
    for (idx, wrapped) in wrap_ansi(&cmd_raw, inner_w).into_iter().enumerate() {
        let mut spans = wrapped;
        if let Some(s) = spans.first_mut() {
            s.style = s.style.add_modifier(Modifier::BOLD);
        }
        lines.push(FormattedLine {
            line: box_line(spans, border),
            is_continuation: idx > 0,
        });
    }
    lines.push(spacer());

    // Body with fixed visual tail to prevent bouncing
    if block.lines.is_empty() {
        lines.push(FormattedLine {
            line: box_line(vec![Span::styled(pad_spaces("(no output)", inner_w), dim)], border),
            is_continuation: false,
        });
    } else {
        let mut visual_body: Vec<FormattedLine> = Vec::new();
        for l in &block.lines {
            for (idx, wrapped) in wrap_ansi(l, inner_w).into_iter().enumerate() {
                visual_body.push(FormattedLine {
                    line: box_line(wrapped, border),
                    is_continuation: idx > 0,
                });
            }
        }

        if !block.expanded && visual_body.len() > TAIL_LINES {
            let hidden = visual_body.len() - TAIL_LINES;
            let hint = format!("... {} lines hidden (Ctrl+O to expand) ...", hidden);
            lines.push(FormattedLine {
                line: box_line(vec![Span::styled(pad_spaces(&hint, inner_w), dim)], border),
                is_continuation: false,
            });
            lines.extend(visual_body.into_iter().skip(hidden));
        } else {
            if block.expanded && visual_body.len() > TAIL_LINES {
                lines.push(FormattedLine {
                    line: box_line(
                        vec![Span::styled(pad_spaces("(Ctrl+O to collapse)", inner_w), dim)],
                        border,
                    ),
                    is_continuation: false,
                });
            }
            lines.extend(visual_body);
        }
    }
    lines.push(spacer());

    // Exit code / Cancel
    match block.status {
        CommandStatus::Failed(code) => {
            let msg = format!("Command exited with code {}", code);
            lines.push(FormattedLine {
                line: box_line(
                    vec![Span::styled(
                        pad_spaces(&msg, inner_w),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )],
                    border,
                ),
                is_continuation: false,
            });
            lines.push(spacer());
        }
        CommandStatus::Cancelled => {
            lines.push(FormattedLine {
                line: box_line(
                    vec![Span::styled(
                        pad_spaces("Command cancelled", inner_w),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    )],
                    border,
                ),
                is_continuation: false,
            });
            lines.push(spacer());
        }
        _ => {}
    }

    // Took / Running line
    let dur = block.duration.unwrap_or_else(|| block.started_at.elapsed());
    let status_str = match block.status {
        CommandStatus::Running => format!("Running {:.1}s", dur.as_secs_f64()),
        _ => format!("Took {:.1}s", dur.as_secs_f64()),
    };
    lines.push(FormattedLine {
        line: box_line(vec![Span::styled(pad_spaces(&status_str, inner_w), dim)], border),
        is_continuation: false,
    });
    lines.push(FormattedLine {
        line: box_border('└', '─', '┘', width, border),
        is_continuation: false,
    });
    lines
}

pub fn render_command_output_block(block: &CommandOutputBlock, width: usize) -> Vec<FormattedLine> {
    let title = block.tool.as_deref().map(|t| format!(" Tool: {} ", t));
    render_command_block_with_title(title.as_deref(), &block.command, block, width)
}

fn render_text_card(
    title: &str,
    text: &str,
    color: Color,
    empty_hint: Option<&str>,
    width: usize,
) -> Vec<FormattedLine> {
    let style = Style::default().fg(color);
    let dim = Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM);
    let inner_w = width.saturating_sub(4);
    let mut lines = vec![FormattedLine {
        line: box_titled_border('┌', title, '─', '┐', width, style),
        is_continuation: false,
    }];
    if text.trim().is_empty() {
        let hint = empty_hint.unwrap_or("");
        lines.push(FormattedLine {
            line: box_line(vec![Span::styled(pad_spaces(hint, inner_w), dim)], style),
            is_continuation: false,
        });
    } else {
        for (idx, wrapped) in wrap_ansi(text, inner_w).into_iter().enumerate() {
            lines.push(FormattedLine {
                line: box_line(wrapped, style),
                is_continuation: idx > 0,
            });
        }
    }
    lines.push(FormattedLine {
        line: box_border('└', '─', '┘', width, style),
        is_continuation: false,
    });
    lines
}

pub fn render_agent_thought_block(thought: &str, width: usize) -> Vec<FormattedLine> {
    render_text_card(" Thought ", thought, Color::Magenta, Some("Thinking..."), width)
}

pub fn render_agent_response_block(response: &str, width: usize) -> Vec<FormattedLine> {
    render_text_card(" Agent ", response, Color::Cyan, None, width)
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
}
