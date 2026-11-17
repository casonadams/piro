use std::future::poll_fn;
use std::task::{Context, Poll, Waker};

use tokio::sync::mpsc;

use crate::agent::AgentEvent;
use crate::sys::RunnerMsg;

struct Receivers {
    runner: mpsc::UnboundedReceiver<RunnerMsg>,
    agent: mpsc::UnboundedReceiver<AgentEvent>,
}

/// Senders the session hands to spawned processes and agents.
pub struct Channels {
    pub runner: mpsc::UnboundedSender<RunnerMsg>,
    pub agent: mpsc::UnboundedSender<AgentEvent>,
    swap: mpsc::UnboundedSender<Receivers>,
}

pub enum Incoming {
    Runner(RunnerMsg),
    Agent(AgentEvent),
}

/// The receiving end of both channels, drained by one consumer.
pub struct Inbox {
    current: Receivers,
    swap: mpsc::UnboundedReceiver<Receivers>,
}

struct Opened {
    runner: mpsc::UnboundedSender<RunnerMsg>,
    agent: mpsc::UnboundedSender<AgentEvent>,
    receivers: Receivers,
}

fn open() -> Opened {
    let (runner, runner_rx) = mpsc::unbounded_channel();
    let (agent, agent_rx) = mpsc::unbounded_channel();
    Opened {
        runner,
        agent,
        receivers: Receivers {
            runner: runner_rx,
            agent: agent_rx,
        },
    }
}

pub fn channels() -> (Channels, Inbox) {
    let opened = open();
    let (swap_tx, swap) = mpsc::unbounded_channel();
    (
        Channels {
            runner: opened.runner,
            agent: opened.agent,
            swap: swap_tx,
        },
        Inbox {
            current: opened.receivers,
            swap,
        },
    )
}

impl Channels {
    /// Gives later work fresh channels; whatever cancelled work already queued is dropped unread.
    pub fn reset(&mut self) {
        let opened = open();
        self.runner = opened.runner;
        self.agent = opened.agent;
        let _ = self.swap.send(opened.receivers);
    }
}

impl Inbox {
    /// Waits for the next message. The senders live in the session, so a closed channel never ends this.
    pub async fn next(&mut self) -> Incoming {
        poll_fn(|cx| self.poll_next(cx)).await
    }

    pub fn try_next(&mut self) -> Option<Incoming> {
        match self.poll_next(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(incoming) => Some(incoming),
            Poll::Pending => None,
        }
    }

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Incoming> {
        while let Poll::Ready(Some(fresh)) = self.swap.poll_recv(cx) {
            self.current = fresh;
        }
        if let Poll::Ready(Some(msg)) = self.current.runner.poll_recv(cx) {
            return Poll::Ready(Incoming::Runner(msg));
        }
        if let Poll::Ready(Some(event)) = self.current.agent.poll_recv(cx) {
            return Poll::Ready(Incoming::Agent(event));
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_drops_messages_queued_by_cancelled_work() {
        let (mut channels, mut inbox) = channels();
        let stale = channels.agent.clone();
        stale.send(AgentEvent::Finished).unwrap();
        channels.reset();
        let _ = stale.send(AgentEvent::Notice("stale".into()));
        assert!(inbox.try_next().is_none());

        channels.agent.send(AgentEvent::Notice("fresh".into())).unwrap();
        assert!(matches!(inbox.try_next(), Some(Incoming::Agent(AgentEvent::Notice(n))) if n == "fresh"));
        assert!(inbox.try_next().is_none());
    }
}
