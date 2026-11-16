use nsh_base::input;
use nsh_core::element::{KeyOptions, Selection};
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::ui::bridge::Ui;
use crate::ui::palette;

/// The prompt between two rules: no side borders and no prompt glyph.
pub fn composer(ui: Ui) -> impl IntoElement {
    composer_keys(&ui);
    view().flex_col().child(rule()).child(field(ui)).child(rule())
}

/// NSH's grid layout draws no border on a wrapper of a lone control, so the rules are text.
fn rule() -> impl IntoElement {
    text(RULE)
        .number_of_lines(1u32)
        .text_overflow(nsh_core::EllipsizeMode::Clip)
        .text_color(palette::muted())
}

const RULE: &str = "────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────";

fn field(ui: Ui) -> impl IntoElement {
    let composer = ui.composer;
    let caret = ui.caret;
    let selection = ui.selection;
    let submit = ui.clone();
    let field = ui.field;
    let first_layout = std::cell::Cell::new(true);
    input()
        .test_id("composer")
        .element_ref(&field)
        .on_layout(move |_: &Bounds<Points>| {
            if first_layout.replace(false) {
                field.focus();
            }
        })
        .multiline(true)
        .submit_behavior(SubmitBehavior::Submit)
        .value(move || composer.get())
        .selected_range(move || selection.get())
        .placeholder("Message piro, or !cmd / !!cmd to run a shell command")
        .placeholder_color(palette::muted())
        .on_change(move |text: &str| composer.set(text.to_owned()))
        .on_selection_change(move |s: &Selection| caret.set(s.end))
        .on_submit(move |text: &str| submit.submit(text))
        .w_full()
}

/// Up and Down recall history at the first and last line; the field still gets the key to move its caret.
/// Ctrl+J is a line break, as the field does not take it.
fn composer_keys(ui: &Ui) {
    for key in ["up", "ctrl+p"] {
        let ui = ui.clone();
        on_key_with(key, recall_options(), move |_| ui.history_prev());
    }
    for key in ["down", "ctrl+n"] {
        let ui = ui.clone();
        on_key_with(key, recall_options(), move |_| ui.history_next());
    }
    let newline = ui.clone();
    on_key_with(
        "ctrl+j",
        KeyOptions {
            when: When::Always,
            repeat: true,
            pass_through: false,
        },
        move |_| newline.insert_newline(),
    );
}

fn recall_options() -> KeyOptions {
    KeyOptions {
        when: When::Always,
        repeat: true,
        pass_through: true,
    }
}
