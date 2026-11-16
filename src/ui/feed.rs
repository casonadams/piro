use nsh_core::prelude::*;

use crate::ui::bridge::Ui;
use crate::ui::card_text::CardText;
use crate::ui::palette;

/// The feed, bottom-anchored: it follows new output unless the user has scrolled up.
pub fn feed(ui: Ui) -> impl IntoElement {
    let ids = ui.ids;
    let empty = move || ids.with(Vec::is_empty);
    view().flex_1().flex_col().child(when(empty, placeholder)).child(
        virtual_list(move || ids.with(Vec::len), move |at| card(Row { ui: ui.clone(), at }))
            .keys(move || ids.get())
            .alignment(ListAlignment::Bottom)
            .follow_end(px(24.0))
            .flex_1(),
    )
}

fn placeholder() -> impl IntoElement {
    text("No commands yet. Type a prompt, or !cmd / !!cmd, and press Enter.").text_color(palette::muted())
}

/// One recycled list cell. The list reuses a cell for other rows by moving its position, so every
/// binding resolves the feed id from the position when it runs instead of capturing it once.
#[derive(Clone)]
struct Row {
    ui: Ui,
    at: Position,
}

impl Row {
    fn id(&self) -> Option<u64> {
        let index = self.at.index();
        self.ui.ids.with(|ids| ids.get(index).copied())
    }

    fn card(&self) -> CardText {
        self.id().map(|id| self.ui.card(id)).unwrap_or_default()
    }

    fn field(&self, pick: fn(CardText) -> String) -> impl Fn() -> String + use<> {
        let row = self.clone();
        move || pick(row.card())
    }
}

fn card(row: Row) -> AnyElement {
    let accent = {
        let row = row.clone();
        move || palette::accent(row.card().accent)
    };
    view()
        .flex_col()
        .px(px(8.0))
        .border_1()
        .border_color(accent)
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(text(row.field(|c| c.header)).font_weight(FontWeight::BOLD))
                .child(copy_button(row.clone())),
        )
        .child(muted_line(&row, |c| c.hint.unwrap_or_default()))
        .child(text(row.field(|c| c.body)))
        .child(muted_line(&row, |c| c.footer))
        .into_element()
}

/// A muted row that takes no space while its text is empty.
fn muted_line(row: &Row, pick: fn(CardText) -> String) -> impl IntoElement {
    let shown = row.field(pick);
    let line = row.clone();
    when(
        move || !shown().is_empty(),
        move || text(line.field(pick)).text_color(palette::muted()),
    )
}

fn copy_button(row: Row) -> impl IntoElement {
    view()
        .px(px(1.0))
        .on_press(move || {
            if let Some(id) = row.id() {
                row.ui.copy(id);
            }
        })
        .child(text("copy").text_color(palette::muted()))
}
