use nsh_base::dialog;
use nsh_core::prelude::*;

use crate::app::session::Approval;
use crate::sys::sanitize::clean_text;
use crate::ui::bridge::Ui;
use crate::ui::card_text::Accent;
use crate::ui::keys::consuming;
use crate::ui::palette;

pub fn approval_dialog(ui: Ui) -> impl IntoElement {
    let approval = ui.approval;
    when(
        move || approval.get().is_some(),
        move || {
            for (key, answer) in [
                ("y", Approval::Allow),
                ("a", Approval::AlwaysAllow),
                ("n", Approval::Deny),
            ] {
                let ui = ui.clone();
                on_key_with(key, consuming(), move |_| ui.answer_approval(answer));
            }
            let cancel = ui.clone();
            dialog()
                .on_cancel(move || cancel.cancel())
                .label("Allow tool call")
                .p(px(16.0))
                .gap(px(16.0))
                .border(px(1.0))
                .border_color(palette::muted())
                .child(text("Allow this tool call?").font_weight(FontWeight::BOLD))
                .child(text(move || clean_text(&approval.get().unwrap_or_default()).into_owned()).selectable(true))
                .child(
                    text("[y] allow   [a] always allow this tool   [n] deny   [esc] cancel the turn")
                        .text_color(palette::accent(Accent::Cyan)),
                )
        },
    )
}
