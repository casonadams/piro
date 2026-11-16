mod agent;
mod app;
mod provider;
mod sys;
mod ui;

use std::time::Duration;

use agent::AgentEvent;
use app::events::{InputCtx, ScrollCtx, handle_input_key, handle_mouse, handle_scroll_key};
use app::{App, FeedItem, Mode};
use crossterm::event::{Event as CrosstermEvent, EventStream};
use futures_util::StreamExt;
use sys::{RunnerMsg, TerminalGuard};
use tokio::sync::mpsc;

struct LoopChannels<'a> {
    runner_tx: &'a mpsc::UnboundedSender<RunnerMsg>,
    runner_rx: &'a mut mpsc::UnboundedReceiver<RunnerMsg>,
    agent_tx: &'a mpsc::UnboundedSender<AgentEvent>,
    agent_rx: &'a mut mpsc::UnboundedReceiver<AgentEvent>,
}

struct TerminalInput {
    pub event: CrosstermEvent,
    pub width: usize,
}

fn dispatch_terminal_event(app: &mut App, input: TerminalInput, ch: &mut LoopChannels<'_>) {
    match input.event {
        CrosstermEvent::Key(key) => match app.mode {
            Mode::Input => {
                let mut ctx = InputCtx {
                    width: input.width,
                    runner_tx: ch.runner_tx,
                    runner_rx: ch.runner_rx,
                    agent_tx: ch.agent_tx,
                    agent_rx: ch.agent_rx,
                };
                handle_input_key(app, key, &mut ctx);
            }
            Mode::Scroll => {
                let mut ctx = ScrollCtx {
                    width: input.width,
                    runner_rx: ch.runner_rx,
                    agent_rx: ch.agent_rx,
                };
                handle_scroll_key(app, key, &mut ctx);
            }
        },
        CrosstermEvent::Mouse(mouse) => handle_mouse(app, mouse, input.width),
        _ => {}
    }
}

fn init_app() -> Result<App, String> {
    let args = app::cli::parse_args(std::env::args())?;
    if let Some(spec) = &args.model {
        provider::provider_from_spec(spec)?;
    }
    Ok(App::new(app::AppOptions { model_spec: args.model }))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut app = match init_app() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{}", msg);
            std::process::exit(1);
        }
    };

    let (mut terminal, _guard) = TerminalGuard::init()?;
    let mut reader = EventStream::new();
    let (runner_tx, mut runner_rx) = mpsc::unbounded_channel::<RunnerMsg>();
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let mut ticker = tokio::time::interval(Duration::from_millis(50));

    while app.running {
        let term_width = terminal.size()?.width as usize;
        terminal.draw(|f| ui::draw(f, &mut app))?;

        tokio::select! {
            biased;
            Some(Ok(event)) = reader.next() => {
                let mut ch = LoopChannels {
                    runner_tx: &runner_tx,
                    runner_rx: &mut runner_rx,
                    agent_tx: &agent_tx,
                    agent_rx: &mut agent_rx,
                };
                dispatch_terminal_event(
                    &mut app,
                    TerminalInput {
                        event,
                        width: term_width,
                    },
                    &mut ch,
                );
            }
            Some(msg) = runner_rx.recv() => handle_runner_msg(&mut app, msg, &agent_tx),
            Some(event) = agent_rx.recv() => handle_agent_event(&mut app, event),
            _ = ticker.tick() => {}
        }
    }
    Ok(())
}

fn handle_runner_msg(app: &mut App, msg: RunnerMsg, agent_tx: &mpsc::UnboundedSender<AgentEvent>) {
    match msg {
        RunnerMsg::Line(line) => app.append_output_line(line),
        RunnerMsg::Done(code, dur) => {
            app.complete_command(code, dur);
            if let Some(cmd) = app.pending_agent_cmd.take() {
                let output = app
                    .feed
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        FeedItem::CommandOutput(b) => Some(b.lines.join("\n")),
                        _ => None,
                    })
                    .unwrap_or_default();
                let prompt = format!("Executed '{}'. Output:\n{}", cmd, output);
                app.spawn_agent_prompt(prompt, agent_tx);
            }
        }
    }
}

fn handle_agent_event(app: &mut App, event: AgentEvent) {
    match event {
        AgentEvent::ThoughtDelta(chunk) => app.append_agent_thought_delta(chunk),
        AgentEvent::ResponseDelta(chunk) => app.append_agent_response_delta(chunk),
        AgentEvent::ToolStart(call) => app.start_agent_tool(call),
        AgentEvent::ToolOutput { call_id, line } => app.append_tool_line(&call_id, line),
        AgentEvent::ToolDone { call_id, exit } => app.complete_tool(&call_id, exit),
        AgentEvent::Committed(msg) => app.commit_agent_message(msg),
        AgentEvent::Notice(notice) => app.feed.push(FeedItem::AgentResponse(notice)),
        AgentEvent::Finished | AgentEvent::Cancelled => app.finish_agent(),
        AgentEvent::Error(err) => {
            app.add_agent_response(format!("Error: {}", err));
            app.finish_agent();
        }
    }
}
