use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Mode};

pub fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match app.mode {
        Mode::Input => {
            let (label, color) = if app.is_command_running() {
                (" RUNNING ", Color::Yellow)
            } else {
                (" READY ", Color::Green)
            };

            let status = Span::styled(
                label,
                Style::default().bg(color).fg(Color::Black).add_modifier(Modifier::BOLD),
            );

            let mut spans = vec![status];
            if app.copied_flash {
                spans.push(Span::styled(
                    "  [Copied to clipboard!]",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ));
            } else if !app.scroll.auto_scroll {
                let msg = format!(
                    "  [Scroll: {} lines up (PgDn / End to return)]",
                    app.scroll.scroll_from_bottom
                );
                spans.push(Span::styled(
                    msg,
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
            }

            Line::from(spans)
        }
        Mode::Scroll => {
            if app.selection_anchor.is_some() {
                let badge = Span::styled(
                    " VISUAL ",
                    Style::default()
                        .bg(Color::Yellow)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
                let hints = Span::styled(
                    " [hjkl/arrows] Extend  [y/Enter] Copy  [v/Space/Esc] Cancel",
                    Style::default().fg(Color::White),
                );
                Line::from(vec![badge, hints])
            } else {
                let badge = Span::styled(
                    " SCROLL ",
                    Style::default()
                        .bg(Color::Cyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
                let hints = Span::styled(
                    " [hjkl/arrows] Move  [v/Space] Select  [b] Copy block  [i/q/Esc] Return",
                    Style::default().fg(Color::White),
                );
                let mut spans = vec![badge, hints];
                if app.copied_flash {
                    spans.push(Span::styled(
                        "  [Copied to clipboard!]",
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                    ));
                }
                Line::from(spans)
            }
        }
    };

    let p = Paragraph::new(line).style(Style::default().bg(Color::Black).fg(Color::White));
    frame.render_widget(p, area);
}
