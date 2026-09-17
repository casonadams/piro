use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use tokio::sync::mpsc;

use crate::agent::AgentEvent;
use crate::app::{App, InputIntent, Mode, SelectionType, parse_input_intent};
use crate::sys::{RunnerMsg, copy, spawn_command};
use crate::ui;

pub struct InputCtx<'a> {
    pub runner_tx: &'a mpsc::UnboundedSender<RunnerMsg>,
    pub runner_rx: &'a mut mpsc::UnboundedReceiver<RunnerMsg>,
    pub agent_tx: &'a mpsc::UnboundedSender<AgentEvent>,
    pub agent_rx: &'a mut mpsc::UnboundedReceiver<AgentEvent>,
}

pub struct ScrollCtx<'a> {
    pub width: usize,
    pub runner_rx: &'a mut mpsc::UnboundedReceiver<RunnerMsg>,
    pub agent_rx: &'a mut mpsc::UnboundedReceiver<AgentEvent>,
}

pub fn cancel_and_drain(
    app: &mut App,
    runner_rx: &mut mpsc::UnboundedReceiver<RunnerMsg>,
    agent_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
) {
    if app.cancel_active() {
        while runner_rx.try_recv().is_ok() {}
        while agent_rx.try_recv().is_ok() {}
    }
}

pub fn copy_and_flash(app: &mut App, width: usize) {
    let text = ui::feed::extract_selected_text(app, width);
    if !text.is_empty() {
        copy(&text);
        app.copied_flash = true;
    }
    app.clear_selection();
}

pub fn submit_input(
    app: &mut App,
    runner_tx: &mpsc::UnboundedSender<RunnerMsg>,
    agent_tx: &mpsc::UnboundedSender<AgentEvent>,
) {
    let val = app.input.take_value();
    if val.trim() == "/clear" {
        app.clear_feed_and_agent_context();
        return;
    }
    let Some(intent) = parse_input_intent(&val) else {
        return;
    };
    if app.is_busy() {
        app.append_output_line("Task already running, press Esc to cancel.".into());
        return;
    }
    match intent {
        InputIntent::CommandWithAgent(cmd) => {
            app.pending_agent_cmd = Some(cmd.clone());
            app.add_command(cmd.clone(), spawn_command(cmd, runner_tx.clone()));
        }
        InputIntent::DirectCommand(cmd) => {
            app.add_command(cmd.clone(), spawn_command(cmd, runner_tx.clone()));
        }
        InputIntent::AgentPrompt(prompt) => {
            app.spawn_agent_prompt(prompt, agent_tx);
        }
    }
}

fn handle_input_ctrl(app: &mut App, code: KeyCode, ctx: &mut InputCtx) {
    match code {
        KeyCode::Char('c') => {
            cancel_and_drain(app, ctx.runner_rx, ctx.agent_rx);
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
            app.clear_feed_and_agent_context();
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
        KeyCode::Char('j') | KeyCode::Enter => app.input.insert('\n'),
        _ => {}
    }
}

fn handle_input_alt(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('b') => app.input.move_prev_word(),
        KeyCode::Char('f') => app.input.move_next_word(),
        KeyCode::Backspace => app.input.delete_prev_word(),
        _ => {}
    }
}

fn handle_input_plain(app: &mut App, code: KeyCode, ctx: &mut InputCtx) {
    match code {
        KeyCode::Esc => {
            if app.is_busy() {
                cancel_and_drain(app, ctx.runner_rx, ctx.agent_rx);
            } else if app.input.value().is_empty() {
                app.enter_scroll_mode();
            } else {
                let _ = app.input.take_value();
            }
        }
        KeyCode::Up => {
            if !app.input.move_up() {
                app.history_prev();
            }
        }
        KeyCode::Down => {
            if !app.input.move_down() {
                app.history_next();
            }
        }
        KeyCode::PageUp => {
            app.enter_scroll_mode();
            app.move_cursor_up(app.viewport_height.max(1));
        }
        KeyCode::PageDown => {
            let _ = app.move_cursor_down(app.viewport_height.max(1));
        }
        KeyCode::Enter => submit_input(app, ctx.runner_tx, ctx.agent_tx),
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

pub fn handle_input_key(app: &mut App, key: crossterm::event::KeyEvent, ctx: &mut InputCtx) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        handle_input_ctrl(app, key.code, ctx);
    } else if key.modifiers.contains(KeyModifiers::ALT) {
        handle_input_alt(app, key.code);
    } else {
        handle_input_plain(app, key.code, ctx);
    }
}

fn handle_scroll_ctrl(app: &mut App, code: KeyCode, ctx: &mut ScrollCtx) {
    let half = (app.viewport_height / 2).max(1);
    match code {
        KeyCode::Char('c') => {
            if app.is_busy() {
                cancel_and_drain(app, ctx.runner_rx, ctx.agent_rx);
            }
            if app.selection_anchor.is_some() {
                app.clear_selection();
            } else {
                app.exit_scroll_mode();
            }
        }
        KeyCode::Char('o') => app.toggle_expand(),
        KeyCode::Char('u') => app.move_cursor_up(half),
        KeyCode::Char('d') => {
            let _ = app.move_cursor_down(half);
        }
        _ => {}
    }
}

struct NavBounds {
    max_col: u16,
    page: usize,
}

fn handle_scroll_movement(app: &mut App, code: KeyCode, b: NavBounds) -> bool {
    match code {
        KeyCode::Left | KeyCode::Char('h') => app.move_cursor_left(1),
        KeyCode::Right | KeyCode::Char('l') => app.move_cursor_right(1, b.max_col),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor_up(1),
        KeyCode::Down | KeyCode::Char('j') => {
            let _ = app.move_cursor_down(1);
        }
        KeyCode::Char('0') => app.cursor.col = 2,
        KeyCode::Char('$') => app.cursor.col = b.max_col.saturating_sub(2),
        KeyCode::PageUp => app.move_cursor_up(b.page),
        KeyCode::PageDown => {
            let _ = app.move_cursor_down(b.page);
        }
        KeyCode::Home | KeyCode::Char('g') => app.scroll_to_top(),
        KeyCode::End | KeyCode::Char('G') => app.scroll_to_bottom(),
        _ => return false,
    }
    true
}

fn handle_scroll_yank(app: &mut App, code: KeyCode, width: usize) {
    if app.selection_anchor.is_none() {
        app.selection_type = SelectionType::Line;
        app.selection_anchor = Some(app.cursor);
    }
    copy_and_flash(app, width);
    if code == KeyCode::Enter {
        app.exit_scroll_mode();
    }
}

fn handle_scroll_plain(app: &mut App, code: KeyCode, ctx: &mut ScrollCtx) {
    let b = NavBounds {
        max_col: ctx.width.saturating_sub(1) as u16,
        page: app.viewport_height.max(1),
    };
    if handle_scroll_movement(app, code, b) {
        return;
    }
    match code {
        KeyCode::Char('v') | KeyCode::Char(' ') => app.toggle_selection(SelectionType::Char),
        KeyCode::Char('V') => app.toggle_selection(SelectionType::Line),
        KeyCode::Char('o') => app.toggle_expand(),
        KeyCode::Char('b') => {
            if let Some(text) = app.copy_current_block() {
                copy(&text);
                app.copied_flash = true;
            }
        }
        KeyCode::Char('y') | KeyCode::Enter => handle_scroll_yank(app, code, ctx.width),
        KeyCode::Esc => {
            if app.selection_anchor.is_some() {
                app.clear_selection();
            } else if app.is_busy() {
                cancel_and_drain(app, ctx.runner_rx, ctx.agent_rx);
            } else {
                app.exit_scroll_mode();
            }
        }
        KeyCode::Char('q') | KeyCode::Char('i') => {
            app.clear_selection();
            app.exit_scroll_mode();
        }
        _ => {}
    }
}

pub fn handle_scroll_key(app: &mut App, key: crossterm::event::KeyEvent, ctx: &mut ScrollCtx) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        handle_scroll_ctrl(app, key.code, ctx);
    } else {
        handle_scroll_plain(app, key.code, ctx);
    }
}

pub fn handle_mouse(app: &mut App, mouse: MouseEvent, width: usize) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_ctrl_j_inserts_newline_and_arrow_navigation() {
        let mut app = App::default();
        let (runner_tx, mut runner_rx) = mpsc::unbounded_channel();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        let mut ctx = InputCtx {
            runner_tx: &runner_tx,
            runner_rx: &mut runner_rx,
            agent_tx: &agent_tx,
            agent_rx: &mut agent_rx,
        };

        app.input.insert('a');
        handle_input_key(&mut app, make_key(KeyCode::Char('j'), KeyModifiers::CONTROL), &mut ctx);
        app.input.insert('b');

        assert_eq!(app.input.value(), "a\nb");
        assert_eq!(app.input.line_count(), 2);
        assert_eq!(app.input.cursor_coords(), (1, 1));

        // Up moves to line 0
        handle_input_key(&mut app, make_key(KeyCode::Up, KeyModifiers::NONE), &mut ctx);
        assert_eq!(app.input.cursor_coords(), (1, 0));

        // Down moves back to line 1
        handle_input_key(&mut app, make_key(KeyCode::Down, KeyModifiers::NONE), &mut ctx);
        assert_eq!(app.input.cursor_coords(), (1, 1));
    }
}
