mod agent;
mod app;
mod clipboard;
mod events;
mod runner;
mod terminal;
mod ui;

use std::time::Duration;

use agent::{AgentEvent, MockAgentBackend, ToolRegistry, spawn_agent};
use app::{App, FeedItem, Mode};
use crossterm::event::{Event as CrosstermEvent, EventStream};
use events::{handle_input_key, handle_mouse, handle_scroll_key};
use futures_util::StreamExt;
use runner::RunnerMsg;
use terminal::TerminalGuard;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let (mut terminal, _guard) = TerminalGuard::init()?;
    let mut app = App::new();
    let mut reader = EventStream::new();
    let (runner_tx, mut runner_rx) = mpsc::unbounded_channel::<RunnerMsg>();
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let mut ticker = tokio::time::interval(Duration::from_millis(50));

    while app.running {
        let size = terminal.size()?;
        let term_width = size.width as usize;

        terminal.draw(|f| ui::draw(f, &mut app))?;

        tokio::select! {
            biased;
            Some(Ok(event)) = reader.next() => {
                match event {
                    CrosstermEvent::Key(key) => match app.mode {
                        Mode::Input => handle_input_key(&mut app, key, &runner_tx, &mut runner_rx, &agent_tx, &mut agent_rx),
                        Mode::Scroll => handle_scroll_key(&mut app, key, term_width, &mut runner_rx, &mut agent_rx),
                    },
                    CrosstermEvent::Mouse(mouse) => handle_mouse(&mut app, mouse, term_width),
                    _ => {}
                }
            }
            Some(msg) = runner_rx.recv() => match msg {
                RunnerMsg::Line(line) => app.append_output_line(line),
                RunnerMsg::Done(code, dur) => {
                    app.complete_command(code, dur);
                    if let Some(cmd) = app.pending_agent_cmd.take() {
                        let output = app.feed.iter().rev().find_map(|item| match item {
                            FeedItem::CommandOutput(b) => Some(b.lines.join("\n")),
                            _ => None,
                        }).unwrap_or_default();
                        let prompt = format!("Executed '{}'. Output:\n{}", cmd, output);
                        let handle = spawn_agent(prompt, MockAgentBackend::new(), ToolRegistry::with_defaults(), agent_tx.clone());
                        app.active_agent = Some(handle);
                    }
                }
            },
            Some(event) = agent_rx.recv() => match event {
                AgentEvent::Thought(thought) => app.add_agent_thought(thought),
                AgentEvent::ToolStart { name, input } => app.start_agent_tool(name, input),
                AgentEvent::ToolOutput(line) => app.append_agent_tool_line(line),
                AgentEvent::ToolDone { success, exit_code, duration } => {
                    app.complete_agent_tool(success, exit_code, duration);
                }
                AgentEvent::Response(resp) => app.add_agent_response(resp),
                AgentEvent::Finished | AgentEvent::Cancelled => app.finish_agent(),
                AgentEvent::Error(err) => {
                    app.add_agent_response(format!("Error: {}", err));
                    app.finish_agent();
                }
            },
            _ = ticker.tick() => {}
        }
    }
    Ok(())
}
