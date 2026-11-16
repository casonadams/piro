use crate::app::feed::{CommandOutputBlock, CommandStatus, FeedItem, TAIL_LINES};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Accent {
    Cyan,
    Magenta,
    Yellow,
    Green,
    Red,
    #[default]
    Muted,
}

/// The words and lines of one feed card, ready to draw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardText {
    pub accent: Accent,
    pub header: String,
    pub hint: Option<String>,
    pub body: String,
    pub footer: String,
}

pub fn card_text(item: &FeedItem) -> CardText {
    match item {
        FeedItem::UserInput(text) => CardText {
            accent: Accent::Cyan,
            header: "You".into(),
            hint: None,
            body: text.clone(),
            footer: String::new(),
        },
        FeedItem::AgentThought(text) => CardText {
            accent: Accent::Magenta,
            header: "Thought".into(),
            hint: None,
            body: if text.is_empty() {
                "Thinking...".into()
            } else {
                text.clone()
            },
            footer: String::new(),
        },
        FeedItem::AgentResponse(text) => CardText {
            accent: Accent::Cyan,
            header: "Agent".into(),
            hint: None,
            body: text.clone(),
            footer: String::new(),
        },
        FeedItem::CommandOutput(block) => command_text(block),
    }
}

fn command_text(block: &CommandOutputBlock) -> CardText {
    let (hidden, lines) = block.visible();
    let hint = if hidden > 0 {
        Some(format!("... {hidden} lines hidden (Ctrl+O to expand) ..."))
    } else if block.expanded && block.lines.len() > TAIL_LINES {
        Some("(Ctrl+O to collapse)".into())
    } else {
        None
    };
    let body = if lines.is_empty() && block.status != CommandStatus::Running {
        "(no output)".into()
    } else {
        lines.join("\n")
    };
    CardText {
        accent: status_accent(&block.status),
        header: match &block.tool {
            Some(tool) => format!("{tool}: {}", block.command),
            None => format!("$ {}", block.command),
        },
        hint,
        body,
        footer: status_footer(block),
    }
}

fn status_accent(status: &CommandStatus) -> Accent {
    match status {
        CommandStatus::Running => Accent::Yellow,
        CommandStatus::Success(_) => Accent::Green,
        CommandStatus::Failed(_) => Accent::Red,
        CommandStatus::Cancelled => Accent::Muted,
    }
}

fn status_footer(block: &CommandOutputBlock) -> String {
    let state = match block.status {
        CommandStatus::Running => "Running".to_owned(),
        CommandStatus::Success(_) => "Done".to_owned(),
        CommandStatus::Failed(code) => format!("Command exited with code {code}"),
        CommandStatus::Cancelled => "Command cancelled".to_owned(),
    };
    match block.duration {
        Some(d) => format!("{state}, took {:.1}s", d.as_secs_f64()),
        None => state,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::agent::tool::ToolExit;

    fn block(lines: usize) -> CommandOutputBlock {
        let mut b = CommandOutputBlock::new("seq".into(), None);
        for i in 1..=lines {
            b.push_line(format!("line {i}"));
        }
        b
    }

    #[test]
    fn long_output_collapses_to_tail_with_hint() {
        let text = card_text(&FeedItem::CommandOutput(block(20)));
        assert_eq!(text.hint.as_deref(), Some("... 12 lines hidden (Ctrl+O to expand) ..."));
        assert_eq!(text.body.lines().count(), TAIL_LINES);
        assert!(text.body.starts_with("line 13\n"));
        assert!(text.body.ends_with("line 20"));
    }

    #[test]
    fn expanded_output_shows_everything_and_offers_collapse() {
        let mut b = block(20);
        b.expanded = true;
        let text = card_text(&FeedItem::CommandOutput(b));
        assert_eq!(text.body.lines().count(), 20);
        assert_eq!(text.hint.as_deref(), Some("(Ctrl+O to collapse)"));
    }

    #[test]
    fn short_output_has_no_hint() {
        let text = card_text(&FeedItem::CommandOutput(block(3)));
        assert_eq!(text.hint, None);
        assert_eq!(text.body, "line 1\nline 2\nline 3");
    }

    #[test]
    fn status_drives_accent_and_footer() {
        let mut b = block(1);
        b.complete(ToolExit {
            code: Some(2),
            duration: Duration::from_millis(1500),
        });
        let text = card_text(&FeedItem::CommandOutput(b));
        assert_eq!(text.accent, Accent::Red);
        assert_eq!(text.footer, "Command exited with code 2, took 1.5s");

        let mut cancelled = block(0);
        cancelled.cancel();
        let text = card_text(&FeedItem::CommandOutput(cancelled));
        assert_eq!(text.accent, Accent::Muted);
        assert_eq!(text.body, "(no output)");
    }

    #[test]
    fn running_command_without_output_has_empty_body() {
        let text = card_text(&FeedItem::CommandOutput(block(0)));
        assert_eq!(text.accent, Accent::Yellow);
        assert_eq!(text.body, "");
    }

    #[test]
    fn tool_blocks_title_with_tool_name() {
        let mut b = CommandOutputBlock::new("ls".into(), Some("bash".into()));
        b.complete(ToolExit {
            code: Some(0),
            duration: Duration::from_secs(0),
        });
        let text = card_text(&FeedItem::CommandOutput(b));
        assert_eq!(text.header, "bash: ls");
        assert_eq!(text.accent, Accent::Green);
    }

    #[test]
    fn agent_cards_keep_their_line_breaks() {
        let text = card_text(&FeedItem::AgentResponse("a\nb".into()));
        assert_eq!(text.accent, Accent::Cyan);
        assert_eq!(text.body, "a\nb");
        let thought = card_text(&FeedItem::AgentThought(String::new()));
        assert_eq!(thought.accent, Accent::Magenta);
        assert_eq!(thought.body, "Thinking...");
    }
}
