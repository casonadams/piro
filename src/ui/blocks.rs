use ansi_to_tui::IntoText;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use crate::app::{CommandOutputBlock, CommandStatus};

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

    let mut lines = Vec::new();
    lines.push(FormattedLine {
        line: box_border('┌', '─', '┐', width, style),
        is_continuation: false,
    });

    for (idx, wrapped) in wrap_text_to_spans(command, inner_w, Style::default().fg(Color::White))
        .into_iter()
        .enumerate()
    {
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
    let empty_line = " ".repeat(inner_w);

    let mut lines = Vec::new();
    lines.push(FormattedLine {
        line: box_border('┌', '─', '┐', width, border),
        is_continuation: false,
    });

    // Command line (word-wrapped if long)
    let cmd_raw = format!("$ {}", block.command);
    for (idx, wrapped) in wrap_text_to_spans(
        &cmd_raw,
        inner_w,
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )
    .into_iter()
    .enumerate()
    {
        lines.push(FormattedLine {
            line: box_line(wrapped, border),
            is_continuation: idx > 0,
        });
    }

    // Spacer
    lines.push(FormattedLine {
        line: box_line(vec![Span::raw(empty_line.clone())], border),
        is_continuation: false,
    });

    // Output body with word-boundary wrapping and ANSI color parsing
    if block.lines.is_empty() {
        lines.push(FormattedLine {
            line: box_line(vec![Span::styled(fit_to_width("(no output)", inner_w), dim)], border),
            is_continuation: false,
        });
    } else {
        for l in &block.lines {
            for (idx, wrapped) in parse_and_wrap_ansi_line(l, inner_w).into_iter().enumerate() {
                lines.push(FormattedLine {
                    line: box_line(wrapped, border),
                    is_continuation: idx > 0,
                });
            }
        }
    }

    // Spacer
    lines.push(FormattedLine {
        line: box_line(vec![Span::raw(empty_line.clone())], border),
        is_continuation: false,
    });

    // Failure / Cancel message
    match block.status {
        CommandStatus::Failed(code) => {
            let msg = format!("Command exited with code {}", code);
            lines.push(FormattedLine {
                line: box_line(
                    vec![Span::styled(
                        fit_to_width(&msg, inner_w),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )],
                    border,
                ),
                is_continuation: false,
            });
            lines.push(FormattedLine {
                line: box_line(vec![Span::raw(empty_line)], border),
                is_continuation: false,
            });
        }
        CommandStatus::Cancelled => {
            let msg = "Command cancelled";
            lines.push(FormattedLine {
                line: box_line(
                    vec![Span::styled(
                        fit_to_width(msg, inner_w),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    )],
                    border,
                ),
                is_continuation: false,
            });
            lines.push(FormattedLine {
                line: box_line(vec![Span::raw(empty_line)], border),
                is_continuation: false,
            });
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
        line: box_line(vec![Span::styled(fit_to_width(&status_str, inner_w), dim)], border),
        is_continuation: false,
    });

    lines.push(FormattedLine {
        line: box_border('└', '─', '┘', width, border),
        is_continuation: false,
    });
    lines
}

pub fn parse_and_wrap_ansi_line(raw: &str, target_width: usize) -> Vec<Vec<Span<'static>>> {
    let spans: Vec<Span<'static>> = match raw.as_bytes().into_text() {
        Ok(text) => text
            .lines
            .into_iter()
            .flat_map(|l| l.spans)
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect(),
        Err(_) => vec![Span::raw(raw.to_string())],
    };

    let tokens = tokenize_spans(spans);
    wrap_tokens_to_lines(tokens, target_width)
}

fn wrap_text_to_spans(text: &str, target_width: usize, style: Style) -> Vec<Vec<Span<'static>>> {
    let tokens = tokenize_spans(vec![Span::styled(text.to_string(), style)]);
    wrap_tokens_to_lines(tokens, target_width)
}

struct Token {
    text: String,
    style: Style,
    is_space: bool,
}

fn tokenize_spans(spans: Vec<Span<'static>>) -> Vec<Token> {
    let mut tokens = Vec::new();
    for span in spans {
        let style = span.style;
        let mut cur_token = String::new();
        let mut in_space: Option<bool> = None;

        for c in span.content.chars() {
            let is_ws = c.is_whitespace();
            match in_space {
                Some(ws) if ws == is_ws => {
                    cur_token.push(c);
                }
                Some(ws) => {
                    tokens.push(Token {
                        text: std::mem::take(&mut cur_token),
                        style,
                        is_space: ws,
                    });
                    cur_token.push(c);
                    in_space = Some(is_ws);
                }
                None => {
                    cur_token.push(c);
                    in_space = Some(is_ws);
                }
            }
        }
        if !cur_token.is_empty() {
            tokens.push(Token {
                text: cur_token,
                style,
                is_space: in_space.unwrap_or(false),
            });
        }
    }
    tokens
}

fn wrap_tokens_to_lines(tokens: Vec<Token>, target_width: usize) -> Vec<Vec<Span<'static>>> {
    if tokens.is_empty() {
        return vec![vec![Span::raw(" ".repeat(target_width))]];
    }

    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut cur_line: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0;

    for token in tokens {
        let token_w: usize = token
            .text
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();

        if token.is_space {
            if cur_w == 0 {
                continue;
            }
            if cur_w + token_w <= target_width {
                cur_w += token_w;
                cur_line.push(Span::styled(token.text, token.style));
            } else {
                cur_line.push(Span::raw(" ".repeat(target_width - cur_w)));
                lines.push(std::mem::take(&mut cur_line));
                cur_w = 0;
            }
        } else if cur_w + token_w <= target_width {
            cur_w += token_w;
            cur_line.push(Span::styled(token.text, token.style));
        } else {
            if cur_w > 0 {
                cur_line.push(Span::raw(" ".repeat(target_width - cur_w)));
                lines.push(std::mem::take(&mut cur_line));
                cur_w = 0;
            }

            if token_w <= target_width {
                cur_w = token_w;
                cur_line.push(Span::styled(token.text, token.style));
            } else {
                let mut chunk = String::new();
                let mut chunk_w = 0;
                for c in token.text.chars() {
                    let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                    if chunk_w + cw > target_width {
                        lines.push(vec![Span::styled(std::mem::take(&mut chunk), token.style)]);
                        chunk_w = 0;
                    }
                    chunk_w += cw;
                    chunk.push(c);
                }
                if !chunk.is_empty() {
                    cur_w = chunk_w;
                    cur_line.push(Span::styled(chunk, token.style));
                }
            }
        }
    }

    if cur_w > 0 {
        cur_line.push(Span::raw(" ".repeat(target_width - cur_w)));
        lines.push(cur_line);
    }

    if lines.is_empty() {
        lines.push(vec![Span::raw(" ".repeat(target_width))]);
    }

    lines
}

pub fn fit_to_width(text: &str, target: usize) -> String {
    let mut w = 0;
    let mut out = String::new();
    for c in text.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > target {
            break;
        }
        w += cw;
        out.push(c);
    }
    if w < target {
        out.push_str(&" ".repeat(target - w));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_wrapping() {
        let raw = "The quick brown fox jumps over the lazy dog";
        let wrapped = parse_and_wrap_ansi_line(raw, 16);
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

        assert_eq!(wrapped[0][0].content, "The");
        assert_eq!(wrapped[0][2].content, "quick");
    }

    #[test]
    fn test_ansi_parsing_and_width() {
        let raw = "\x1b[32mhello\x1b[0m \x1b[31mworld\x1b[0m";
        let wrapped = parse_and_wrap_ansi_line(raw, 20);
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0][0].content, "hello");
        assert_eq!(wrapped[0][0].style.fg, Some(Color::Green));
    }
}
