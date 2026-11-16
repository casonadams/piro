use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use nsh_base::{dialog, input};
use nsh_core::element::KeyOptions;
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::auth::claude::{self, LoginRequest};
use crate::sys::sanitize::clean_text;
use crate::ui::bridge::Ui;
use crate::ui::card_text::Accent;
use crate::ui::palette;

/// The `/login` dialog: the authorize URL, a field for the pasted code, and Claude Code import.
#[derive(Clone)]
pub struct Login {
    pub open: Signal<bool>,
    url: Signal<String>,
    status: Signal<String>,
    code: Signal<String>,
    request: Rc<RefCell<Option<Arc<LoginRequest>>>>,
}

impl Login {
    pub fn new() -> Self {
        Self {
            open: signal(false),
            url: signal(String::new()),
            status: signal(String::new()),
            code: signal(String::new()),
            request: Rc::default(),
        }
    }

    /// Starts a fresh PKCE login and opens the browser on its authorize URL.
    pub fn start(&self) {
        let request = claude::begin_login();
        self.status.set(match claude::open_browser(&request.url) {
            Ok(()) => "Opened your browser. Sign in, then paste the code below.".into(),
            Err(e) => format!("Could not open a browser ({e}); open the URL yourself."),
        });
        self.url.set(request.url.clone());
        self.code.set(String::new());
        *self.request.borrow_mut() = Some(Arc::new(request));
        self.open.set(true);
    }

    fn close(&self) {
        self.open.set(false);
        self.request.borrow_mut().take();
    }

    fn finish(&self, ui: Ui, pasted: String) {
        let Some(request) = self.request.borrow().clone() else {
            return;
        };
        self.status.set("Exchanging the code...".into());
        let login = self.clone();
        on_runtime(
            async move { claude::complete_login(&request, &pasted).await },
            move |result| match result {
                Ok(email) => login.succeed(&ui, signed_in_text(email)),
                Err(e) => login.status.set(format!("Sign-in failed: {e}")),
            },
        );
    }

    fn import(&self, ui: Ui) {
        self.status.set("Looking for Claude Code credentials...".into());
        let login = self.clone();
        on_runtime(claude::import_local_credentials(), move |result| match result {
            Ok(true) => login.succeed(&ui, "Imported existing Claude Code credentials.".into()),
            Ok(false) => login
                .status
                .set("No Claude Code credentials found on this machine.".into()),
            Err(e) => login.status.set(format!("Import failed: {e}")),
        });
    }

    fn succeed(&self, ui: &Ui, text: String) {
        self.close();
        ui.with(|s| s.notice(text));
    }
}

fn signed_in_text(email: Option<String>) -> String {
    match email {
        Some(email) => format!("Logged in to Claude as {email}. Use --model claude/<model>."),
        None => "Logged in to Claude. Use --model claude/<model>.".into(),
    }
}

/// Runs `work` on the tokio runtime and hands its output to `done` on the engine thread.
fn on_runtime<T: Send + 'static>(work: impl Future<Output = T> + Send + 'static, done: impl FnOnce(T) + 'static) {
    let handle = crate::sys::runtime().spawn(work);
    drop(spawn(async move {
        if let Ok(value) = handle.await {
            done(value);
        }
    }));
}

pub fn login_dialog(ui: Ui) -> impl IntoElement {
    let login = ui.login.clone();
    let (open, url, status) = (login.open, login.url, login.status);
    when(open, move || {
        let cancel = login.clone();
        let escape = login.clone();
        let consume = KeyOptions {
            when: When::Always,
            repeat: false,
            pass_through: false,
        };
        on_key_with("escape", consume, move |_| escape.close());
        dialog()
            .on_cancel(move || cancel.close())
            .label("Sign in to Claude")
            .p(px(16.0))
            .gap(px(16.0))
            .border(px(1.0))
            .border_color(palette::muted())
            .child(text("Sign in to Claude").font_weight(FontWeight::BOLD))
            .child(text(move || url.get()).selectable(true))
            .child(code_field(ui.clone()))
            .child(text(move || clean_text(&status.get()).into_owned()).text_color(palette::muted()))
            .child(actions(ui.clone()))
    })
}

fn code_field(ui: Ui) -> impl IntoElement {
    let code = ui.login.code;
    input()
        .test_id("login-code")
        .auto_focus(true)
        .submit_behavior(SubmitBehavior::Submit)
        .value(move || code.get())
        .placeholder("Paste the authorization code (or CODE#STATE)")
        .placeholder_color(palette::muted())
        .on_change(move |text: &str| code.set(text.to_owned()))
        .on_submit(move |text: &str| ui.login.finish(ui.clone(), text.trim().to_owned()))
        .w_full()
}

fn actions(ui: Ui) -> impl IntoElement {
    let import = ui.clone();
    let cancel = ui.login.clone();
    view()
        .flex_row()
        .gap(px(24.0))
        .child(action("Import Claude Code credentials", move || {
            import.login.import(import.clone())
        }))
        .child(action("Cancel", move || cancel.close()))
}

fn action(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .on_press(on_press)
        .child(text(label).text_color(palette::accent(Accent::Cyan)))
}
