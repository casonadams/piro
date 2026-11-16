//! The terminal session: raw mode, the alternate screen and input modes, restored on drop and from a panic.

mod caps;
mod input;

use std::io::{self, BufWriter, IsTerminal, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::{cursor, event, execute, terminal};
use nsh_platform_tui::{CursorStyle, RunEvent, TerminalCapabilities};
use ratatui_crossterm::CrosstermBackend;

pub use caps::TermEnv;
use caps::parse_replies;

pub type Backend = CrosstermBackend<BufWriter<Stdout>>;

/// The default foreground (OSC 10) and background (OSC 11), the kitty keyboard flags and DA1 last:
/// DA1 is answered by every terminal, so its answer ends the wait.
const QUERY: &[u8] = b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[?u\x1b[c";
const QUERY_TIMEOUT: Duration = Duration::from_secs(1);
const KEYBOARD_FLAGS: event::KeyboardEnhancementFlags = event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
    .union(event::KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
    .union(event::KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS);
/// crossterm's poll truncates to whole milliseconds on Linux and spins through the rest:
/// there each poll is brief and `run_app` sleeps out its wait.
const DECODE_POLL: Duration = Duration::from_micros(50);

/// Whether raw mode and the alternate screen are ours to restore.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Whether the kitty keyboard flags were pushed and must be popped.
static KEYBOARD: AtomicBool = AtomicBool::new(false);

/// Owns the terminal until dropped. Its capabilities come from the startup queries.
pub struct Session {
    capabilities: TerminalCapabilities,
}

impl Session {
    /// Enters raw mode and the alternate screen, then queries the terminal for its colors and keyboard protocol.
    pub fn enter(env: &TermEnv) -> io::Result<Self> {
        if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "piro needs an interactive terminal on stdin and stdout",
            ));
        }
        install_panic_hook();
        terminal::enable_raw_mode()?;
        ACTIVE.store(true, Ordering::Release);
        // Read before crossterm's reader starts, so the answers are ours to read.
        let replies = parse_replies(&query_replies().unwrap_or_default());
        let keyboard = caps::keyboard_protocol(&replies, env);
        let session = Self {
            capabilities: caps::capabilities(&replies, env),
        };
        if let Err(error) = enter_modes(keyboard) {
            restore();
            return Err(error);
        }
        Ok(session)
    }

    pub fn capabilities(&self) -> TerminalCapabilities {
        self.capabilities.clone()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        restore();
    }
}

/// The ratatui backend writing the frames: buffered, so one frame is one flush.
pub fn backend() -> Backend {
    CrosstermBackend::new(BufWriter::new(io::stdout()))
}

fn enter_modes(keyboard: bool) -> io::Result<()> {
    execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
    input_modes()?;
    if keyboard {
        execute!(io::stdout(), event::PushKeyboardEnhancementFlags(KEYBOARD_FLAGS))?;
        KEYBOARD.store(true, Ordering::Release);
    }
    Ok(())
}

/// Bracketed paste, focus reports and mouse capture. A terminal that switched focus away may have reset them.
pub fn input_modes() -> io::Result<()> {
    execute!(
        io::stdout(),
        event::EnableBracketedPaste,
        event::EnableFocusChange,
        event::EnableMouseCapture
    )
}

/// Writes the cursor shape a frame asks for: a steady bar while a text field is edited.
pub fn set_cursor_style(style: CursorStyle) -> io::Result<()> {
    let shape = match style {
        CursorStyle::Bar => cursor::SetCursorStyle::SteadyBar,
        _ => cursor::SetCursorStyle::DefaultUserShape,
    };
    execute!(io::stdout(), shape)
}

/// Waits up to `timeout` for one terminal event and converts it. Idle when none arrived.
pub fn poll(timeout: Duration) -> io::Result<RunEvent> {
    if event::poll(poll_wait(timeout))? {
        Ok(input::convert(event::read()?))
    } else {
        Ok(RunEvent::Idle)
    }
}

#[cfg(target_os = "macos")]
fn poll_wait(timeout: Duration) -> Duration {
    timeout.max(DECODE_POLL)
}

#[cfg(not(target_os = "macos"))]
fn poll_wait(_timeout: Duration) -> Duration {
    DECODE_POLL
}

/// Restores the terminal once: a no-op when no session is active. Best-effort, as it runs from drop and panics.
fn restore() {
    if !ACTIVE.swap(false, Ordering::AcqRel) {
        return;
    }
    if KEYBOARD.swap(false, Ordering::AcqRel) {
        let _ = execute!(io::stdout(), event::PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        io::stdout(),
        event::DisableMouseCapture,
        event::DisableFocusChange,
        event::DisableBracketedPaste,
        cursor::SetCursorStyle::DefaultUserShape,
        terminal::LeaveAlternateScreen,
        cursor::Show
    );
    let _ = terminal::disable_raw_mode();
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// Writes the query and reads the answers until DA1 arrives or the timeout passes.
fn query_replies() -> io::Result<Vec<u8>> {
    let mut out = io::stdout();
    out.write_all(QUERY)?;
    out.flush()?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let mut replies = Vec::new();
    while !parse_replies(&replies).complete {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        if !stdin_ready(left)? {
            continue;
        }
        let mut chunk = [0u8; 256];
        let read = read_stdin(&mut chunk)?;
        if read == 0 {
            break;
        }
        replies.extend_from_slice(&chunk[..read]);
    }
    Ok(replies)
}

fn stdin_ready(wait: Duration) -> io::Result<bool> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = wait.as_millis().clamp(1, libc::c_int::MAX as u128) as libc::c_int;
    // SAFETY: fd is one initialized pollfd, live for this bounded wait.
    let ready = unsafe { libc::poll(&mut fd, 1, millis) };
    if ready < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(error)
        };
    }
    Ok(ready > 0)
}

/// Reads the descriptor directly: std's stdin buffer would hold back bytes crossterm must see.
fn read_stdin(chunk: &mut [u8]) -> io::Result<usize> {
    // SAFETY: chunk is writable for its length, and the descriptor is stdin.
    let read = unsafe { libc::read(libc::STDIN_FILENO, chunk.as_mut_ptr().cast(), chunk.len()) };
    usize::try_from(read).map_err(|_| io::Error::last_os_error())
}
