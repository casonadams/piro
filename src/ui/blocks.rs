use ansi_to_tui::IntoText;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use crate::app::{CommandOutputBlock, CommandStatus};

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

pub fn render_user_input_block(command: &str, width: usize) -> Vec<Line<'static>> {
    let style = Style::default().fg(Color::Cyan);
    let inner_w = width.saturating_sub(4);
    let cmd = fit_to_width(command, inner_w);

    vec![
        box_border('┌', '─', '┐', width, style),
        box_line(vec![Span::styled(cmd, Style::default().fg(Color::White))], style),
        box_border('└', '─', '┘', width, style),
    ]
}

pub fn render_command_output_block(block: &CommandOutputBlock, width: usize) -> Vec<Line<'static>> {
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
    lines.push(box_border('┌', '─', '┐', width, border));

    // Command line
    let cmd_text = fit_to_width(&format!("$ {}", block.command), inner_w);
    lines.push(box_line(
        vec![Span::styled(
            cmd_text,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )],
        border,
    ));

    // Spacer
    lines.push(box_line(vec![Span::raw(empty_line.clone())], border));

    // Output body with ANSI color parsing
    if block.lines.is_empty() {
        lines.push(box_line(
            vec![Span::styled(fit_to_width("(no output)", inner_w), dim)],
            border,
        ));
    } else {
        for l in &block.lines {
            let parsed_spans = parse_ansi_line(l, inner_w);
            lines.push(box_line(parsed_spans, border));
        }
    }

    // Spacer
    lines.push(box_line(vec![Span::raw(empty_line.clone())], border));

    // Failure / Cancel message
    match block.status {
        CommandStatus::Failed(code) => {
            let msg = fit_to_width(&format!("Command exited with code {}", code), inner_w);
            lines.push(box_line(
                vec![Span::styled(
                    msg,
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )],
                border,
            ));
            lines.push(box_line(vec![Span::raw(empty_line)], border));
        }
        CommandStatus::Cancelled => {
            let msg = fit_to_width("Command cancelled", inner_w);
            lines.push(box_line(
                vec![Span::styled(
                    msg,
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )],
                border,
            ));
            lines.push(box_line(vec![Span::raw(empty_line)], border));
        }
        _ => {}
    }

    // Took / Running line
    let dur = block.duration.unwrap_or_else(|| block.started_at.elapsed());
    let status_str = match block.status {
        CommandStatus::Running => format!("Running {:.1}s", dur.as_secs_f64()),
        _ => format!("Took {:.1}s", dur.as_secs_f64()),
    };
    lines.push(box_line(
        vec![Span::styled(fit_to_width(&status_str, inner_w), dim)],
        border,
    ));

    lines.push(box_border('└', '─', '┘', width, border));
    lines
}

fn parse_ansi_line(raw: &str, target_width: usize) -> Vec<Span<'static>> {
    let spans: Vec<Span<'static>> = match raw.as_bytes().into_text() {
        Ok(text) => text
            .lines
            .into_iter()
            .flat_map(|l| l.spans)
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect(),
        Err(_) => vec![Span::raw(raw.to_string())],
    };

    fit_spans_to_width(spans, target_width)
}

fn fit_spans_to_width(spans: Vec<Span<'static>>, target_width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut current_width = 0;

    for span in spans {
        let span_w: usize = span
            .content
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        if current_width + span_w <= target_width {
            current_width += span_w;
            out.push(span);
        } else {
            let budget = target_width.saturating_sub(current_width);
            let mut truncated = String::new();
            let mut w = 0;
            for c in span.content.chars() {
                let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                if w + cw > budget {
                    break;
                }
                w += cw;
                truncated.push(c);
            }
            current_width += w;
            out.push(Span::styled(truncated, span.style));
            break;
        }
    }

    if current_width < target_width {
        out.push(Span::raw(" ".repeat(target_width - current_width)));
    }

    out
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
    fn test_ansi_parsing_and_width() {
        let raw = "\x1b[32mhello\x1b[0m \x1b[31mworld\x1b[0m";
        let spans = parse_ansi_line(raw, 20);
        let total_w: usize = spans
            .iter()
            .map(|s| {
                s.content
                    .chars()
                    .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                    .sum::<usize>()
            })
            .sum();
        assert_eq!(total_w, 20);
        assert_eq!(spans[0].content, "hello");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
    }
}
