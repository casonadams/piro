use nsh_core::element::KeyOptions;
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::ui::bridge::Ui;

/// Ctrl+O toggles the latest output block and Ctrl+L clears the feed.
pub fn global_keys(ui: &Ui) {
    let expand = ui.clone();
    on_key("ctrl+o", move |_| expand.toggle_expand());
    let clear = ui.clone();
    on_key("ctrl+l", move |_| clear.clear());
}

/// Esc and Ctrl+C cancel while work runs. Mounted only while busy, so when idle NSH's own keys stand.
pub fn busy_keys(ui: Ui) -> impl IntoElement {
    let esc = ui.clone();
    on_key_with("escape", consuming(), move |_| esc.cancel());
    on_key_with("ctrl+c", consuming(), move |_| ui.cancel());
    view()
}

fn consuming() -> KeyOptions {
    KeyOptions {
        when: When::Always,
        repeat: false,
        pass_through: false,
    }
}
