use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Mode, SelectionType};

fn input_footer(app: &App) -> Line<'static> {
    let (lbl, col) = if app.is_agent_running() {
        (" THINKING ", Color::Magenta)
    } else if app.is_command_running() {
        (" RUNNING ", Color::Yellow)
    } else {
        (" READY ", Color::Green)
    };
    let mut spans = vec![Span::styled(
        lbl,
        Style::default().bg(col).fg(Color::Black).add_modifier(Modifier::BOLD),
    )];
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

fn scroll_footer(app: &App) -> Line<'static> {
    let (badge, hints) = match app.selection_anchor {
        Some(_) if app.selection_type == SelectionType::Line => (
            " VISUAL LINE ",
            " [jk/arrows] Extend lines  [y/Enter] Copy  [v/V/Esc] Cancel",
        ),
        Some(_) => (
            " VISUAL ",
            " [hjkl/arrows] Extend  [y/Enter] Copy  [v/Space/Esc] Cancel",
        ),
        None => (
            " SCROLL ",
            " [hjkl] Move  [v] Char sel  [V] Line sel  [b] Copy block  [i/q/Esc] Return",
        ),
    };
    let color = if app.selection_anchor.is_some() {
        Color::Yellow
    } else {
        Color::Cyan
    };
    let mut spans = vec![
        Span::styled(
            badge,
            Style::default().bg(color).fg(Color::Black).add_modifier(Modifier::BOLD),
        ),
        Span::styled(hints, Style::default().fg(Color::White)),
    ];
    if app.copied_flash {
        spans.push(Span::styled(
            "  [Copied to clipboard!]",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

pub fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match app.mode {
        Mode::Input => input_footer(app),
        Mode::Scroll => scroll_footer(app),
    };
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(Color::Black)), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_footer_badges() {
        let app = App::default();
        assert!(!app.is_agent_running());
        assert!(!app.is_command_running());
        assert!(!app.is_busy());
    }
}
