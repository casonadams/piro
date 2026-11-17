use nsh_core::prelude::*;

use crate::app::session::{Session, Status};
use crate::ui::bridge::Ui;
use crate::ui::card_text::Accent;
use crate::ui::palette;

/// The status line: the mode badge, then the model and token counts.
pub fn footer(ui: Ui) -> impl IntoElement {
    let (status, line) = (ui.status, ui.status_line);
    view()
        .flex_row()
        .gap(px(8.0))
        .child(
            text(move || badge(status.get()))
                .font_weight(FontWeight::BOLD)
                .text_color(black())
                .bg(move || palette::accent(badge_accent(status.get()))),
        )
        .child(text(move || line.get()).text_color(palette::muted()))
}

pub fn status_line(session: &Session) -> String {
    let t = session.tokens;
    if t == Default::default() {
        return session.model_spec().to_owned();
    }
    format!(
        "{}  in {} out {} cached {}  ctx {}",
        session.model_spec(),
        tokens(t.input + t.cache_write),
        tokens(t.output),
        tokens(t.cache_read),
        tokens(session.context_tokens),
    )
}

fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn badge(status: Status) -> &'static str {
    match status {
        Status::Ready => " READY ",
        Status::Running => " RUNNING ",
        Status::Thinking => " THINKING ",
        Status::Approval => " APPROVE? ",
    }
}

fn badge_accent(status: Status) -> Accent {
    match status {
        Status::Ready => Accent::Green,
        Status::Running => Accent::Yellow,
        Status::Thinking => Accent::Magenta,
        Status::Approval => Accent::Cyan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_shorten_by_magnitude() {
        assert_eq!(tokens(999), "999");
        assert_eq!(tokens(45_230), "45.2k");
        assert_eq!(tokens(3_100_000), "3.1M");
    }
}
