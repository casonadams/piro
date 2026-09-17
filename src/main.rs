mod app;
mod runner;
mod terminal;
mod ui;

use std::time::Duration;

use app::App;
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode, KeyModifiers, MouseEventKind};
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
                        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                            if !app.cancel_active() {
                                app.running = false;
                            }
                        } else {
                            match key.code {
                                KeyCode::Esc => {
                                    if !app.cancel_active() {
                                        app.running = false;
                                    }
                                }
                                KeyCode::PageUp => app.scroll_up(10),
                                KeyCode::PageDown => app.scroll_down(10),
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
                    }
                    CrosstermEvent::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => app.scroll_up(3),
                        MouseEventKind::ScrollDown => app.scroll_down(3),
                        _ => {}
                    },
                    CrosstermEvent::Resize(_, _) => {}
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
