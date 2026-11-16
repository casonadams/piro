mod bridge;
mod card_text;
mod composer;
mod feed;
mod footer;
#[cfg(test)]
mod headless_tests;
mod keys;
mod login;
mod palette;

use nsh_core::prelude::*;

use crate::app::session::Status;
use bridge::Ui;

/// The piro screen: the feed, a blank row, the prompt between rules, and the status line.
#[component]
pub fn App() -> impl IntoElement {
    let ui = Ui::new();
    keys::global_keys(&ui);
    let status = ui.status;
    let busy = {
        let ui = ui.clone();
        move || keys::busy_keys(ui.clone())
    };
    view()
        .flex_1()
        .flex_col()
        .child(when(move || status.get() != Status::Ready, busy))
        .child(feed::feed(ui.clone()))
        .child(text(" "))
        .child(composer::composer(ui.clone()))
        .child(login::login_dialog(ui.clone()))
        .child(footer::footer(ui))
}
