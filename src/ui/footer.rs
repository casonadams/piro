use nsh_core::prelude::*;

use crate::app::session::Status;
use crate::ui::bridge::Ui;
use crate::ui::card_text::Accent;
use crate::ui::palette;

/// The status line: the mode badge, then the model spec.
pub fn footer(ui: Ui) -> impl IntoElement {
    let status = ui.status;
    view()
        .flex_row()
        .gap(px(8.0))
        .child(
            text(move || badge(status.get()))
                .font_weight(FontWeight::BOLD)
                .text_color(black())
                .bg(move || palette::accent(badge_accent(status.get()))),
        )
        .child(text(crate::launch::model_spec()).text_color(palette::muted()))
}

fn badge(status: Status) -> &'static str {
    match status {
        Status::Ready => " READY ",
        Status::Running => " RUNNING ",
        Status::Thinking => " THINKING ",
    }
}

fn badge_accent(status: Status) -> Accent {
    match status {
        Status::Ready => Accent::Green,
        Status::Running => Accent::Yellow,
        Status::Thinking => Accent::Magenta,
    }
}
