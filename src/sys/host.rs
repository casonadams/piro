//! The standalone terminal host: runs an NSH root in the terminal until it exits, is signalled or fails.

use std::cell::{Cell, RefCell};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nsh_core::element::Registry;
use nsh_platform_tui::{AppOptions, AppView, CursorStyle, Input, Limits, Root, RunEvent, run_app};
use ratatui_core::terminal::Terminal;

use super::clipboard::TerminalClipboard;
use super::terminal::{self, Session, TermEnv};
use super::{filelog, signals};

/// Runs `root` with `ui_sets` in the terminal. Logs go to `~/.piro/piro.log`.
pub fn run(root: Root, ui_sets: Vec<Registry>) -> io::Result<()> {
    let data_dir = data_dir()?;
    filelog::init(&data_dir.join("piro.log"))?;
    let _signals = signals::Guard::install()?;
    let session = Session::enter(&TermEnv::from_process())?;
    let mut frontend = Frontend::default();
    let mut options = AppOptions::default();
    options.limits = Limits::standalone();
    options.data_dir = Some(data_dir);
    options.capabilities = session.capabilities();
    options.clipboard = Some(frontend.clipboard.clone());
    options.ui_sets = ui_sets;
    let mut view = AppView::new(options);
    frontend.watch(&mut view);
    let mut screen = Terminal::new(terminal::backend())?;
    // The first layout needs the terminal's size, so one frame is drawn before the app loads.
    screen.draw(|frame| view.render(frame, frame.area()))?;
    view.load(root)?;
    run_app(&mut screen, &mut view, |timeout| frontend.between_frames(timeout))?;
    frontend.take_failure()
}

/// What the loop shares with the app and does between frames: the clipboard, cursor shape, signals and input.
#[derive(Default)]
struct Frontend {
    clipboard: Arc<TerminalClipboard>,
    failure: Rc<RefCell<Option<String>>>,
    cursor: Rc<Cell<CursorStyle>>,
    shown: CursorStyle,
}

impl Frontend {
    /// Records what the app reports: an app error ends the run, and the cursor shape it asks for.
    fn watch(&self, view: &mut AppView) {
        let reported = self.failure.clone();
        view.on_app_error(move |error| *reported.borrow_mut() = Some(error.to_string()));
        let cursor = self.cursor.clone();
        view.on_cursor_style_changed(move |style| cursor.set(style));
    }

    fn between_frames(&mut self, timeout: Duration) -> io::Result<RunEvent> {
        if signals::requested() || self.failure.borrow().is_some() {
            return Ok(RunEvent::Exit);
        }
        self.clipboard.flush()?;
        if self.cursor.get() != self.shown {
            self.shown = self.cursor.get();
            terminal::set_cursor_style(self.shown)?;
        }
        // NSH drops keyboard focus when the surface blurs; piro keeps the composer focused instead.
        match terminal::poll(timeout)? {
            RunEvent::Input(Input::Focus(true)) => {
                terminal::input_modes()?;
                Ok(RunEvent::Redraw)
            }
            RunEvent::Input(Input::Focus(false)) => Ok(RunEvent::Idle),
            event => Ok(event),
        }
    }

    fn take_failure(&self) -> io::Result<()> {
        match self.failure.borrow_mut().take() {
            Some(message) => Err(io::Error::other(message)),
            None => Ok(()),
        }
    }
}

fn data_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let dir = PathBuf::from(home).join(".piro");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
