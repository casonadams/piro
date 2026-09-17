mod app;
mod clipboard;
mod runner;
mod terminal;
mod ui;

use std::time::Duration;

use app::{App, Mode, ScreenPos};
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use futures_util::StreamExt;
use ratatui::layout::Rect;
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
        let size = terminal.size()?;
        let feed_area = Rect::new(0, 0, size.width, size.height.saturating_sub(5));

        terminal.draw(|f| {
            ui::draw(f, &mut app);

            // Scrape buffer cells directly when a copy is triggered, then immediately clear highlight
            if app.pending_copy {
                if let Some(sel) = app.current_selection() {
                    let text = ui::feed::extract_selection_from_buf(f.buffer_mut(), &sel, feed_area);
                    if !text.is_empty() {
                        clipboard::copy_to_clipboard(&text);
                        app.copied_flash = true;
                    }
                }
                app.clear_selection();
                app.pending_copy = false;
            }
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
                        match app.mode {
                            Mode::Input => handle_input_mode_key(&mut app, key, &runner_tx),
                            Mode::Scroll => handle_scroll_mode_key(&mut app, key, feed_area),
                        }
                    }
                    CrosstermEvent::Mouse(mouse) => {
                        handle_mouse_event(&mut app, mouse, feed_area);
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

fn handle_scroll_mode_key(app: &mut App, key: crossterm::event::KeyEvent, feed_area: Rect) {
    let max_col = feed_area.width.saturating_sub(1);
    let max_row = feed_area.height.saturating_sub(1);

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if app.selection_anchor.is_some() {
                    app.clear_selection();
                } else {
                    app.exit_scroll_mode();
                }
            }
            KeyCode::Char('u') => app.move_cursor_up(10),
            KeyCode::Char('d') => app.move_cursor_down(10, max_row),
            _ => {}
        }
        return;
    }

    match key.code {
        // Toggle visual selection (like tmux 'v' / Space)
        KeyCode::Char('v') | KeyCode::Char(' ') => {
            app.toggle_selection();
        }
        // Copy entire command block (convenient 1-key block copy)
        KeyCode::Char('b') => {
            if let Some(text) = app.copy_current_block() {
                clipboard::copy_to_clipboard(&text);
                app.copied_flash = true;
            }
        }
        // Yank / Copy selected lines
        KeyCode::Char('y') | KeyCode::Enter => {
            if app.selection_anchor.is_some() {
                app.pending_copy = true;
            } else {
                // Copy current line
                app.selection_anchor = Some(ScreenPos {
                    col: 2,
                    row: app.cursor_pos.row,
                });
                app.cursor_pos.col = max_col.saturating_sub(2);
                app.pending_copy = true;
            }
            app.exit_scroll_mode();
        }
        // Exit or cancel selection
        KeyCode::Esc => {
            if app.selection_anchor.is_some() {
                app.clear_selection();
            } else {
                app.exit_scroll_mode();
            }
        }
        KeyCode::Char('q') | KeyCode::Char('i') => {
            app.clear_selection();
            app.exit_scroll_mode();
        }
        // 2D Cursor movement in feed (h/j/k/l and arrow keys)
        KeyCode::Left | KeyCode::Char('h') => app.move_cursor_left(1),
        KeyCode::Right | KeyCode::Char('l') => app.move_cursor_right(1, max_col),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor_up(1),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor_down(1, max_row),
        KeyCode::Char('0') => app.cursor_pos.col = 2,
        KeyCode::Char('$') => app.cursor_pos.col = max_col.saturating_sub(2),
        KeyCode::PageUp => app.move_cursor_up(10),
        KeyCode::PageDown => app.move_cursor_down(10, max_row),
        KeyCode::Home | KeyCode::Char('g') => app.scroll_to_top(),
        KeyCode::End | KeyCode::Char('G') => app.scroll_to_bottom(),
        _ => {}
    }
}

fn handle_mouse_event(app: &mut App, mouse: crossterm::event::MouseEvent, feed_area: Rect) {
    let max_row = feed_area.height.saturating_sub(1);

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if app.mode == Mode::Input {
                app.enter_scroll_mode();
            }
            app.move_cursor_up(2);
        }
        MouseEventKind::ScrollDown => {
            app.move_cursor_down(2, max_row);
            if app.scroll.auto_scroll || app.scroll.scroll_from_bottom == 0 {
                app.exit_scroll_mode();
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if mouse.row < feed_area.bottom() {
                app.enter_scroll_mode();
                let pos = ScreenPos {
                    col: mouse.column,
                    row: mouse.row,
                };
                app.cursor_pos = pos;
                app.selection_anchor = Some(pos);
                app.pending_copy = false;
            }
        }
        MouseEventKind::Drag(MouseButton::Left) if app.mode == Mode::Scroll => {
            app.cursor_pos = ScreenPos {
                col: mouse.column,
                row: mouse.row,
            };
        }
        MouseEventKind::Up(MouseButton::Left) if app.mode == Mode::Scroll => {
            if let Some(anchor) = app.selection_anchor {
                if anchor != app.cursor_pos {
                    // Triggers scraping from frame buffer on next render, then clears selection immediately
                    app.pending_copy = true;
                } else {
                    app.clear_selection();
                }
            }
        }
        _ => {}
    }
}
