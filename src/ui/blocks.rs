use ansi_to_tui::IntoText;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use crate::app::{CommandOutputBlock, CommandStatus};

pub const TAIL_LINES: usize = 8;

pub struct FormattedLine {
    pub line: Line<'static>,
    pub is_continuation: bool,
}

fn box_border(left: char, fill: char, right: char, width: usize, style: Style) -> Line<'static> {
    let inner = fill.to_string().repeat(width.saturating_sub(2));
    Line::from(vec![Span::styled(format!("{}{}{}", left, inner, right), style)])
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
    let mut out = vec![FormattedLine {
        line: box_border('┌', '─', '┐', width, style),
        is_continuation: false,
    }];
    for (i, l) in wrap_ansi(command, inner_w).into_iter().enumerate() {
        out.push(FormattedLine {
            line: box_line(l, style),
            is_continuation: i > 0,
        });
    }
    out.push(FormattedLine {
        line: box_border('└', '─', '┘', width, style),
        is_continuation: false,
    });
    out
}

pub fn render_command_output_block(block: &CommandOutputBlock, width: usize) -> Vec<FormattedLine> {
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

    let mut out = vec![FormattedLine {
        line: box_border('┌', '─', '┐', width, border),
        is_continuation: false,
    }];

    // Header $ <command>
    let cmd_header = format!("$ {}", block.command);
    for (i, l) in wrap_ansi(&cmd_header, inner_w).into_iter().enumerate() {
        let mut spans = l;
        if let Some(s) = spans.first_mut() {
            s.style = s.style.add_modifier(Modifier::BOLD);
        }
        out.push(FormattedLine {
            line: box_line(spans, border),
            is_continuation: i > 0,
        });
    }
    out.push(spacer());

    // Body lines with fixed-height live tailing to eliminate bounce
    if block.lines.is_empty() {
        out.push(FormattedLine {
            line: box_line(vec![Span::styled(pad_spaces("(no output)", inner_w), dim)], border),
            is_continuation: false,
        });
    } else {
        let mut visual_body: Vec<FormattedLine> = Vec::new();
        for l in &block.lines {
            for (i, wrapped) in wrap_ansi(l, inner_w).into_iter().enumerate() {
                visual_body.push(FormattedLine {
                    line: box_line(wrapped, border),
                    is_continuation: i > 0,
                });
            }
        }

        // Live tail stability: lock to exactly TAIL_LINES visual lines so height never fluctuates
        if !block.expanded && visual_body.len() > TAIL_LINES {
            let hidden = visual_body.len() - TAIL_LINES;
            let hint = format!("... {} lines hidden (Ctrl+O to expand) ...", hidden);
            out.push(FormattedLine {
                line: box_line(vec![Span::styled(pad_spaces(&hint, inner_w), dim)], border),
                is_continuation: false,
            });
            out.extend(visual_body.into_iter().skip(hidden));
        } else {
            if block.expanded && visual_body.len() > TAIL_LINES {
                out.push(FormattedLine {
                    line: box_line(
                        vec![Span::styled(pad_spaces("(Ctrl+O to collapse)", inner_w), dim)],
                        border,
                    ),
                    is_continuation: false,
                });
            }
            out.extend(visual_body);
        }
    }

    out.push(spacer());

    // Exit code / Cancel
    match block.status {
        CommandStatus::Failed(code) => {
            let msg = format!("Command exited with code {}", code);
            out.push(FormattedLine {
                line: box_line(
                    vec![Span::styled(
                        pad_spaces(&msg, inner_w),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )],
                    border,
                ),
                is_continuation: false,
            });
            out.push(spacer());
        }
        CommandStatus::Cancelled => {
            out.push(FormattedLine {
                line: box_line(
                    vec![Span::styled(
                        pad_spaces("Command cancelled", inner_w),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    )],
                    border,
                ),
                is_continuation: false,
            });
            out.push(spacer());
        }
        _ => {}
    }

    // Took / Running line
    let dur = block.duration.unwrap_or_else(|| block.started_at.elapsed());
    let status_str = match block.status {
        CommandStatus::Running => format!("Running {:.1}s", dur.as_secs_f64()),
        _ => format!("Took {:.1}s", dur.as_secs_f64()),
    };
    out.push(FormattedLine {
        line: box_line(vec![Span::styled(pad_spaces(&status_str, inner_w), dim)], border),
        is_continuation: false,
    });

    out.push(FormattedLine {
        line: box_border('└', '─', '┘', width, border),
        is_continuation: false,
    });
    out
}

pub fn pad_spaces(s: &str, target_w: usize) -> String {
    let w: usize = s.chars().map(|c| UnicodeWidthChar::width(c).unwrap_or(0)).sum();
    if w < target_w {
        format!("{}{}", s, " ".repeat(target_w - w))
    } else {
        s.to_string()
    }
}

pub fn wrap_ansi(raw: &str, target_w: usize) -> Vec<Vec<Span<'static>>> {
    let spans: Vec<Span<'static>> = match raw.as_bytes().into_text() {
        Ok(t) => t
            .lines
            .into_iter()
            .flat_map(|l| l.spans)
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect(),
        Err(_) => vec![Span::raw(raw.to_string())],
    };

    let total: usize = spans
        .iter()
        .map(|s| {
            s.content
                .chars()
                .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                .sum::<usize>()
        })
        .sum();

    if total <= target_w {
        let mut line = spans;
        if total < target_w {
            line.push(Span::raw(" ".repeat(target_w - total)));
        }
        return vec![line];
    }

    // Word boundary wrap
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut cur_w = 0;

    for span in spans {
        let style = span.style;
        for word in span.content.split_inclusive(char::is_whitespace) {
            let ww: usize = word.chars().map(|c| UnicodeWidthChar::width(c).unwrap_or(0)).sum();
            if cur_w + ww <= target_w {
                cur.push(Span::styled(word.to_string(), style));
                cur_w += ww;
            } else {
                if cur_w > 0 {
                    cur.push(Span::raw(" ".repeat(target_w - cur_w)));
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
                let trimmed = word.trim_start();
                let tw: usize = trimmed.chars().map(|c| UnicodeWidthChar::width(c).unwrap_or(0)).sum();
                if tw <= target_w {
                    cur.push(Span::styled(trimmed.to_string(), style));
                    cur_w = tw;
                } else {
                    for c in trimmed.chars() {
                        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                        if cur_w + cw > target_w {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                        cur.push(Span::styled(c.to_string(), style));
                        cur_w += cw;
                    }
                }
            }
        }
    }
    if cur_w > 0 {
        cur.push(Span::raw(" ".repeat(target_w - cur_w)));
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_wrapping() {
        let raw = "The quick brown fox jumps over the lazy dog";
        let wrapped = wrap_ansi(raw, 16);
        assert!(wrapped.len() > 1);

        for line in &wrapped {
            let line_w: usize = line
                .iter()
                .map(|s| {
                    s.content
                        .chars()
                        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                        .sum::<usize>()
                })
                .sum();
            assert_eq!(line_w, 16);
        }
    }

    #[test]
    fn test_ansi_parsing_and_width() {
        let raw = "\x1b[32mhello\x1b[0m \x1b[31mworld\x1b[0m";
        let wrapped = wrap_ansi(raw, 20);
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0][0].content, "hello");
        assert_eq!(wrapped[0][0].style.fg, Some(Color::Green));
    }
}
