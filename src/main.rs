mod app;
mod clipboard;
mod runner;
mod terminal;
mod ui;

use std::time::Duration;

use app::{App, Mode, SelectionType};
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
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
        let size = terminal.size()?;
        let term_width = size.width as usize;

        terminal.draw(|f| {
            ui::draw(f, &mut app);
        })?;

        tokio::select! {
            biased;
            Some(Ok(event)) = reader.next() => {
                match event {
                    CrosstermEvent::Key(key) => match app.mode {
                        Mode::Input => handle_input_key(&mut app, key, &runner_tx, &mut runner_rx),
                        Mode::Scroll => handle_scroll_key(&mut app, key, term_width, &mut runner_rx),
                    },
                    CrosstermEvent::Mouse(mouse) => handle_mouse(&mut app, mouse, term_width),
                    CrosstermEvent::Resize(_, _) => {}
                    _ => {}
                }
            }
            Some(msg) = runner_rx.recv() => match msg {
                RunnerMsg::Line(line) => app.append_output_line(line),
                RunnerMsg::Done(code, dur) => app.complete_command(code, dur),
            },
            _ = ticker.tick() => {}
        }
    }
    Ok(())
}

fn cancel_and_drain(app: &mut App, rx: &mut mpsc::UnboundedReceiver<RunnerMsg>) {
    if app.cancel_active() {
        while rx.try_recv().is_ok() {}
    }
}

fn copy_and_flash(app: &mut App, width: usize) {
    let text = ui::feed::extract_selected_text(app, width);
    if !text.is_empty() {
        clipboard::copy(&text);
        app.copied_flash = true;
    }
    app.clear_selection();
}

fn handle_input_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    tx: &mpsc::UnboundedSender<RunnerMsg>,
    rx: &mut mpsc::UnboundedReceiver<RunnerMsg>,
) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                cancel_and_drain(app, rx);
                app.input.take_value();
            }
            KeyCode::Char('d') => {
                if app.input.value().is_empty() {
                    app.running = false;
                } else {
                    app.input.delete_forward();
                }
            }
            KeyCode::Char('l') => {
                app.feed.clear();
                app.scroll_to_bottom();
            }
            KeyCode::Char('s') => app.enter_scroll_mode(),
            KeyCode::Char('o') => app.toggle_expand(),
            KeyCode::Char('w') => app.input.delete_prev_word(),
            KeyCode::Char('p') => app.history_prev(),
            KeyCode::Char('n') => app.history_next(),
            KeyCode::Char('a') => app.input.move_start(),
            KeyCode::Char('e') => app.input.move_end(),
            KeyCode::Char('u') => app.input.delete_line(),
            KeyCode::Char('k') => app.input.delete_to_end(),
            _ => {}
        }
        return;
    }

    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Char('b') => app.input.move_prev_word(),
            KeyCode::Char('f') => app.input.move_next_word(),
            KeyCode::Backspace => app.input.delete_prev_word(),
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => {
            if app.is_command_running() {
                cancel_and_drain(app, rx);
            } else if app.input.value().is_empty() {
                app.enter_scroll_mode();
            } else {
                let _ = app.input.take_value();
            }
        }
        KeyCode::Up => app.history_prev(),
        KeyCode::Down => app.history_next(),
        KeyCode::PageUp => {
            app.enter_scroll_mode();
            let page = app.viewport_height.max(1);
            app.move_cursor_up(page);
        }
        KeyCode::PageDown => {
            let page = app.viewport_height.max(1);
            let _ = app.move_cursor_down(page);
        }
        KeyCode::Enter => {
            let cmd = app.input.take_value().trim().to_string();
            if !cmd.is_empty() {
                if app.is_command_running() {
                    app.append_output_line("Command already running, press Esc to cancel.".into());
                } else {
                    let handle = spawn_command(cmd.clone(), tx.clone());
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

fn handle_scroll_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    width: usize,
    rx: &mut mpsc::UnboundedReceiver<RunnerMsg>,
) {
    let max_col = width.saturating_sub(1) as u16;
    let page = app.viewport_height.max(1);
    let half_page = (app.viewport_height / 2).max(1);

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if app.is_command_running() {
                    cancel_and_drain(app, rx);
                }
                if app.selection_anchor.is_some() {
                    app.clear_selection();
                } else {
                    app.exit_scroll_mode();
                }
            }
            KeyCode::Char('o') => app.toggle_expand(),
            KeyCode::Char('u') => app.move_cursor_up(half_page),
            KeyCode::Char('d') => {
                let _ = app.move_cursor_down(half_page);
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('v') | KeyCode::Char(' ') => app.toggle_selection(SelectionType::Char),
        KeyCode::Char('V') => app.toggle_selection(SelectionType::Line),
        KeyCode::Char('o') => app.toggle_expand(),
        KeyCode::Char('b') => {
            if let Some(text) = app.copy_current_block() {
                clipboard::copy(&text);
                app.copied_flash = true;
            }
        }
        KeyCode::Char('y') => {
            if app.selection_anchor.is_none() {
                app.selection_type = SelectionType::Line;
                app.selection_anchor = Some(app.cursor);
            }
            copy_and_flash(app, width);
        }
        KeyCode::Enter => {
            if app.selection_anchor.is_none() {
                app.selection_type = SelectionType::Line;
                app.selection_anchor = Some(app.cursor);
            }
            copy_and_flash(app, width);
            app.exit_scroll_mode();
        }
        KeyCode::Esc => {
            if app.selection_anchor.is_some() {
                app.clear_selection();
            } else if app.is_command_running() {
                cancel_and_drain(app, rx);
            } else {
                app.exit_scroll_mode();
            }
        }
        KeyCode::Char('q') | KeyCode::Char('i') => {
            app.clear_selection();
            app.exit_scroll_mode();
        }
        KeyCode::Left | KeyCode::Char('h') => app.move_cursor_left(1),
        KeyCode::Right | KeyCode::Char('l') => app.move_cursor_right(1, max_col),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor_up(1),
        KeyCode::Down | KeyCode::Char('j') => {
            let _ = app.move_cursor_down(1);
        }
        KeyCode::Char('0') => app.cursor.col = 2,
        KeyCode::Char('$') => app.cursor.col = max_col.saturating_sub(2),
        KeyCode::PageUp => app.move_cursor_up(page),
        KeyCode::PageDown => {
            let _ = app.move_cursor_down(page);
        }
        KeyCode::Home | KeyCode::Char('g') => app.scroll_to_top(),
        KeyCode::End | KeyCode::Char('G') => app.scroll_to_bottom(),
        _ => {}
    }
}

fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent, width: usize) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if app.mode == Mode::Input {
                app.enter_scroll_mode();
            }
            app.move_cursor_up(2);
        }
        MouseEventKind::ScrollDown => {
            if app.mode == Mode::Scroll && app.move_cursor_down(2) {
                app.exit_scroll_mode();
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(pos) = app.screen_coords_to_doc_pos(mouse.column, mouse.row) {
                app.enter_scroll_mode();
                app.selection_type = SelectionType::Char;
                app.cursor = pos;
                app.selection_anchor = Some(pos);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) if app.mode == Mode::Scroll => {
            if let Some(pos) = app.screen_coords_to_doc_pos(mouse.column, mouse.row) {
                app.cursor = pos;
            }
        }
        MouseEventKind::Up(MouseButton::Left) if app.mode == Mode::Scroll => {
            if let Some(anchor) = app.selection_anchor {
                if anchor != app.cursor {
                    copy_and_flash(app, width);
                } else {
                    app.clear_selection();
                }
            }
        }
        _ => {}
    }
}
