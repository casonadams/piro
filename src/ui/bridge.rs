use std::cell::RefCell;
use std::rc::Rc;

use nsh_core::prelude::*;

use crate::app::channels::{Inbox, channels};
use crate::app::feed::FeedItem;
use crate::app::history::{byte_index, caret_on_first_line, caret_on_last_line, load_history};
use crate::app::session::{Session, Status, UiRequest};
use crate::ui::card_text::{CardText, card_text};
use crate::ui::login::Login;

/// The session on the engine thread, mirrored into signals after every change.
#[derive(Clone)]
pub struct Ui {
    session: Rc<RefCell<Session>>,
    pub ids: Signal<Vec<u64>>,
    pub revision: Signal<u64>,
    pub status: Signal<Status>,
    pub composer: Signal<String>,
    /// The caret, in UTF-16 units, as the field reports it.
    pub caret: Signal<u32>,
    /// The selection the field is shown with. Written only when the app moves the caret itself.
    pub selection: Signal<(u32, u32)>,
    pub field: ElementRef,
    pub login: Login,
}

impl Ui {
    /// Starts the session and the task that applies engine and process output to it.
    pub fn new() -> Self {
        let (channels, inbox) = channels();
        let session = Session::new(crate::launch::model_spec(), load_history(), channels);
        let ui = Self {
            session: Rc::new(RefCell::new(session)),
            ids: signal(Vec::new()),
            revision: signal(0),
            status: signal(Status::Ready),
            composer: signal(String::new()),
            caret: signal(0),
            selection: signal_with((0, 0), SignalEquals::Never),
            field: element_ref(),
            login: Login::new(),
        };
        ui.drain(inbox);
        ui
    }

    /// Applies every message that has arrived as one batch, so a burst of streamed tokens costs one redraw.
    fn drain(&self, mut inbox: Inbox) {
        let ui = self.clone();
        let task = spawn(async move {
            loop {
                let first = inbox.next().await;
                ui.with(|s| {
                    s.apply(first);
                    while let Some(next) = inbox.try_next() {
                        s.apply(next);
                    }
                });
            }
        });
        cleanup(move || drop(task));
    }

    /// Changes the session, then tells the screen what changed.
    pub fn with<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let result = f(&mut self.session.borrow_mut());
        self.sync();
        result
    }

    pub fn read<R>(&self, f: impl FnOnce(&Session) -> R) -> R {
        f(&self.session.borrow())
    }

    /// The card for `id`, tracked: its readers run again when the session changes.
    pub fn card(&self, id: u64) -> CardText {
        self.revision.get();
        self.read(|s| s.feed.get(id).map(card_text).unwrap_or_default())
    }

    fn sync(&self) {
        let (ids, status) = {
            let s = self.session.borrow();
            (s.feed.ids(), s.status())
        };
        self.ids.set(ids);
        self.status.set(status);
        self.revision.update(|r| *r += 1);
    }

    pub fn submit(&self, text: &str) {
        self.composer.set(String::new());
        if self.with(|s| s.submit(text)) == Some(UiRequest::Login) {
            self.login.start();
        }
    }

    pub fn cancel(&self) {
        self.with(|s| {
            s.cancel();
        });
    }

    pub fn clear(&self) {
        self.with(|s| s.clear());
    }

    pub fn toggle_expand(&self) {
        self.with(|s| s.toggle_expand());
    }

    pub fn copy(&self, id: u64) {
        if let Some(text) = self.read(|s| s.feed.get(id).map(FeedItem::copy_text)) {
            set_clipboard(text);
        }
    }

    /// Inserts a line break at the caret and leaves the caret after it.
    pub fn insert_newline(&self) {
        let mut text = self.composer.get_untracked();
        let caret = self.caret.get_untracked();
        text.insert(byte_index(&text, caret), '\n');
        self.composer.set(text);
        self.selection.set((caret + 1, caret + 1));
        self.caret.set(caret + 1);
    }

    /// Recalls the previous entry, when the caret is on the first line.
    pub fn history_prev(&self) {
        let text = self.composer.get_untracked();
        if !caret_on_first_line(&text, self.caret.get_untracked()) {
            return;
        }
        let prev = self.session.borrow_mut().history.older(&text);
        if let Some(prev) = prev {
            self.composer.set(prev);
        }
    }

    /// Recalls the next entry, when the caret is on the last line.
    pub fn history_next(&self) {
        let text = self.composer.get_untracked();
        if !caret_on_last_line(&text, self.caret.get_untracked()) {
            return;
        }
        let next = self.session.borrow_mut().history.newer();
        if let Some(next) = next {
            self.composer.set(next);
        }
    }
}
