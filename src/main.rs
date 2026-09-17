mod app;
mod clipboard;
mod runner;
mod terminal;
mod ui;

use std::time::Duration;

use app::{App, Mode};
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use runner::{RunnerMsg, spawn_command};
use terminal::TerminalGuard;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let (mut terminal, _guard) = TerminalGuard::init()?;
    let mut app = App::new();
    let mut reader = EventStream::new();
    let (runner_tx, mut runner_rx) = mpsc::unbounded_channel::<RunnerMsg>();
    let mut ticker = tokio::time::interval(Duration::from_millis(50));

    while app.running {
        terminal.draw(|f| {
            ui::draw(f, &mut app);
        })?;

        tokio::select! {
            _ = ticker.tick() => {}
            Some(msg) = runner_rx.recv() => {
                match msg {
                    RunnerMsg::Line(line) => app.append_output_line(line),
                    RunnerMsg::Done(code, dur) => app.complete_command(code, dur),
                }
            }
            Some(Ok(event)) = reader.next() => {
                match event {
                    CrosstermEvent::Key(key) => {
                        let term_width = terminal.size()?.width as usize;
                        match app.mode {
                            Mode::Input => handle_input_mode_key(&mut app, key, &runner_tx),
                            Mode::Scroll => handle_scroll_mode_key(&mut app, key, term_width),
                        }
                    }
                    CrosstermEvent::Resize(_, _) => {}
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn handle_input_mode_key(app: &mut App, key: crossterm::event::KeyEvent, runner_tx: &mpsc::UnboundedSender<RunnerMsg>) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if !app.cancel_active() {
                    app.running = false;
                }
            }
            KeyCode::Char('s') => {
                app.enter_scroll_mode();
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => {
            if !app.cancel_active() {
                if app.input.value().is_empty() {
                    app.enter_scroll_mode();
                } else {
                    let _ = app.input.take_value();
                }
            }
        }
        KeyCode::PageUp => {
            app.enter_scroll_mode();
            app.move_cursor_up(10);
        }
        KeyCode::PageDown => {
            app.scroll_down(10);
        }
        KeyCode::Enter => {
            let cmd = app.input.take_value().trim().to_string();
            if !cmd.is_empty() {
                if app.is_command_running() {
                    app.append_output_line("Command already running, press Ctrl+C to cancel.".into());
                } else {
                    let handle = spawn_command(cmd.clone(), runner_tx.clone());
                    app.add_command(cmd, handle);
                }
            }
        }
        KeyCode::Char(c) => app.input.insert(c),
        KeyCode::Backspace => app.input.delete_back(),
        KeyCode::Delete => app.input.delete_forward(),
        KeyCode::Left => app.input.move_left(),
        KeyCode::Right => app.input.move_right(),
        KeyCode::Home => app.input.move_start(),
        KeyCode::End => app.input.move_end(),
        _ => {}
    }
}

fn handle_scroll_mode_key(app: &mut App, key: crossterm::event::KeyEvent, term_width: usize) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if app.selection_anchor.is_some() {
                    app.selection_anchor = None;
                } else {
                    app.exit_scroll_mode();
                }
            }
            KeyCode::Char('u') => app.move_cursor_up(10),
            KeyCode::Char('d') => app.move_cursor_down(10),
            _ => {}
        }
        return;
    }

    match key.code {
        // Toggle visual selection (like tmux 'v' / Space)
        KeyCode::Char('v') | KeyCode::Char(' ') => {
            app.toggle_selection();
        }
        // Yank / Copy selected lines
        KeyCode::Char('y') | KeyCode::Enter => {
            let width = term_width.saturating_sub(1);
            let text = ui::feed::get_selected_text(app, width);
            if !text.is_empty() {
                clipboard::copy_to_clipboard(&text);
                app.copied_flash = true;
            }
            app.selection_anchor = None;
        }
        // Exit or cancel selection
        KeyCode::Esc => {
            if app.selection_anchor.is_some() {
                app.selection_anchor = None;
            } else {
                app.exit_scroll_mode();
            }
        }
        KeyCode::Char('q') | KeyCode::Char('i') => {
            app.exit_scroll_mode();
        }
        // Cursor movement in feed
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor_up(1),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor_down(1),
        KeyCode::PageUp => app.move_cursor_up(10),
        KeyCode::PageDown => app.move_cursor_down(10),
        KeyCode::Home | KeyCode::Char('g') => app.scroll_to_top(),
        KeyCode::End | KeyCode::Char('G') => app.scroll_to_bottom(),
        _ => {}
    }
}
