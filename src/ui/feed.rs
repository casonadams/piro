use nsh_core::prelude::*;

use crate::ui::bridge::Ui;
use crate::ui::card_text::CardText;
use crate::ui::palette;

/// The feed, bottom-anchored: it follows new output unless the user has scrolled up.
pub fn feed(ui: Ui) -> impl IntoElement {
    let ids = ui.ids;
    let empty = move || ids.with(Vec::is_empty);
    view().flex_1().flex_col().child(when(empty, placeholder)).child(
        virtual_list(
            move || ids.with(Vec::len),
            move |at| {
                let id = ids.with_untracked(|v| v.get(at.index()).copied());
                match id {
                    Some(id) => card(ui.clone(), id),
                    None => view().into_element(),
                }
            },
        )
        .keys(move || ids.get())
        .alignment(ListAlignment::Bottom)
        .follow_end(px(24.0))
        .flex_1(),
    )
}

fn placeholder() -> impl IntoElement {
    text("No commands yet. Type a prompt, or !cmd / !!cmd, and press Enter.").text_color(palette::muted())
}

fn card(ui: Ui, id: u64) -> AnyElement {
    let accent = {
        let ui = ui.clone();
        move || palette::accent(ui.card(id).accent)
    };
    view()
        .flex_col()
        .px(px(8.0))
        .pb(px(16.0))
        .border_1()
        .border_color(accent)
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(text(field(&ui, id, |c| c.header)).font_weight(FontWeight::BOLD))
                .child(copy_button(ui.clone(), id)),
        )
        .child(text(field(&ui, id, |c| c.hint.unwrap_or_default())).text_color(palette::muted()))
        .child(text(field(&ui, id, |c| c.body)))
        .child(text(field(&ui, id, |c| c.footer)).text_color(palette::muted()))
        .into_element()
}

fn field(ui: &Ui, id: u64, pick: fn(CardText) -> String) -> impl Fn() -> String + use<> {
    let ui = ui.clone();
    move || pick(ui.card(id))
}

fn copy_button(ui: Ui, id: u64) -> impl IntoElement {
    view()
        .px(px(1.0))
        .on_press(move || ui.copy(id))
        .child(text("copy").text_color(palette::muted()))
}
