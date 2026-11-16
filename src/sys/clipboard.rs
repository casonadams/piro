//! The app's clipboard. Copies reach the terminal (OSC 52) between frames, and the system clipboard on macOS.

use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use nsh_core::clipboard::ClipboardBackend;

#[derive(Default)]
pub struct TerminalClipboard {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    copied: Option<String>,
    initialized: bool,
    pending: Option<String>,
}

impl TerminalClipboard {
    /// Sends the text copied since the last call. Called between frames, so no escape splits a frame.
    pub fn flush(&self) -> io::Result<()> {
        let Some(text) = self.lock().pending.take() else {
            return Ok(());
        };
        let mut out = io::stdout();
        write!(out, "\x1b]52;c;{}\x07", STANDARD.encode(&text))?;
        out.flush()?;
        copy_to_pasteboard(&text);
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ClipboardBackend for TerminalClipboard {
    fn text(&self) -> Option<String> {
        // Text copied here is ours; the system clipboard may belong to another machine (SSH).
        let state = self.lock();
        if state.initialized {
            return state.copied.clone();
        }
        drop(state);
        system_text()
    }

    fn set_text(&self, text: Option<String>) {
        let mut state = self.lock();
        state.pending = Some(text.clone().unwrap_or_default());
        state.copied = text;
        state.initialized = true;
    }
}

/// Pipes to `pbcopy`, waited on in the background: the frame loop never blocks on the pasteboard.
#[cfg(target_os = "macos")]
fn copy_to_pasteboard(text: &str) {
    let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(not(target_os = "macos"))]
fn copy_to_pasteboard(_text: &str) {}

#[cfg(target_os = "macos")]
fn system_text() -> Option<String> {
    run_paste(&[("pbpaste", &[])])
}

#[cfg(not(target_os = "macos"))]
fn system_text() -> Option<String> {
    run_paste(&[
        ("wl-paste", &["--no-newline"]),
        ("xclip", &["-selection", "clipboard", "-out"]),
    ])
}

fn run_paste(commands: &[(&str, &[&str])]) -> Option<String> {
    commands.iter().find_map(|(program, args)| {
        let output = Command::new(program)
            .args(*args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).ok())
            .flatten()
    })
}
