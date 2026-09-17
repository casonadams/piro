use std::time::Duration;

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
    lines.push(box_line(
        vec![Span::styled(empty_line.clone(), Style::default())],
        border,
    ));

    // Output body
    if block.lines.is_empty() {
        lines.push(box_line(
            vec![Span::styled(fit_to_width("(no output)", inner_w), dim)],
            border,
        ));
    } else {
        for l in &block.lines {
            lines.push(box_line(
                vec![Span::styled(
                    fit_to_width(l, inner_w),
                    Style::default().fg(Color::White),
                )],
                border,
            ));
        }
    }

    // Spacer
    lines.push(box_line(
        vec![Span::styled(empty_line.clone(), Style::default())],
        border,
    ));

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
            lines.push(box_line(vec![Span::styled(empty_line, Style::default())], border));
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
            lines.push(box_line(vec![Span::styled(empty_line, Style::default())], border));
        }
        _ => {}
    }

    // Took / Running line
    let status_str = match block.status {
        CommandStatus::Running => format!("Running {}", format_duration(block.started_at.elapsed())),
        _ => format!(
            "Took {}",
            format_duration(block.duration.unwrap_or_else(|| block.started_at.elapsed()))
        ),
    };
    lines.push(box_line(
        vec![Span::styled(fit_to_width(&status_str, inner_w), dim)],
        border,
    ));

    lines.push(box_border('└', '─', '┘', width, border));
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

pub fn format_duration(dur: Duration) -> String {
    format!("{:.1}s", dur.as_secs_f64())
}
