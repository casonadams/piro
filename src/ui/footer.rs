use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;

pub fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
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
    if !app.scroll.auto_scroll {
        let msg = format!(
            "  Scrollback active: {} lines up (PgDn / End to return)",
            app.scroll.scroll_from_bottom
        );
        spans.push(Span::styled(
            msg,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }

    let p = Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Black).fg(Color::White));
    frame.render_widget(p, area);
}
