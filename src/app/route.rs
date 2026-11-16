#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputIntent {
    Clear,
    Login,
    DirectCommand(String),
    CommandWithAgent(String),
    AgentPrompt(String),
}

/// What a submitted line does. Blank input routes nowhere.
pub fn parse_input_intent(input: &str) -> Option<InputIntent> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(intent) = slash_command(trimmed) {
        return Some(intent);
    }
    Some(bang_intent(trimmed).unwrap_or_else(|| InputIntent::AgentPrompt(trimmed.to_owned())))
}

fn slash_command(input: &str) -> Option<InputIntent> {
    match input {
        "/clear" => Some(InputIntent::Clear),
        "/login" | "/login claude" => Some(InputIntent::Login),
        _ => None,
    }
}

/// `!!cmd` runs directly and `!cmd` runs then goes to the agent; a bare `!` or `!!` is a plain prompt.
fn bang_intent(input: &str) -> Option<InputIntent> {
    if let Some(cmd) = input.strip_prefix("!!") {
        return non_empty(cmd).map(InputIntent::DirectCommand);
    }
    input
        .strip_prefix('!')
        .and_then(non_empty)
        .map(InputIntent::CommandWithAgent)
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The agent prompt that carries a `!cmd` run's output.
pub fn agent_prompt_for_command(cmd: &str, output: &str) -> String {
    format!("Executed '{cmd}'. Output:\n{output}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_routes_nowhere() {
        assert_eq!(parse_input_intent(""), None);
        assert_eq!(parse_input_intent("   \n "), None);
    }

    #[test]
    fn double_bang_is_direct_command() {
        assert_eq!(
            parse_input_intent("!!echo hi"),
            Some(InputIntent::DirectCommand("echo hi".into()))
        );
        assert_eq!(
            parse_input_intent("  !!  ls -la  "),
            Some(InputIntent::DirectCommand("ls -la".into()))
        );
    }

    #[test]
    fn single_bang_runs_then_asks_agent() {
        assert_eq!(
            parse_input_intent("!git status"),
            Some(InputIntent::CommandWithAgent("git status".into()))
        );
    }

    #[test]
    fn bare_bangs_are_plain_prompts() {
        assert_eq!(parse_input_intent("!"), Some(InputIntent::AgentPrompt("!".into())));
        assert_eq!(parse_input_intent("!!"), Some(InputIntent::AgentPrompt("!!".into())));
        assert_eq!(parse_input_intent("!! "), Some(InputIntent::AgentPrompt("!!".into())));
    }

    #[test]
    fn plain_text_is_agent_prompt() {
        assert_eq!(
            parse_input_intent("How many files are here?"),
            Some(InputIntent::AgentPrompt("How many files are here?".into()))
        );
    }

    #[test]
    fn login_slash_command() {
        assert_eq!(parse_input_intent("/login"), Some(InputIntent::Login));
        assert_eq!(parse_input_intent("/login claude"), Some(InputIntent::Login));
        assert_eq!(
            parse_input_intent("/login openai"),
            Some(InputIntent::AgentPrompt("/login openai".into()))
        );
    }

    #[test]
    fn clear_slash_command() {
        assert_eq!(parse_input_intent("/clear"), Some(InputIntent::Clear));
        assert_eq!(parse_input_intent(" /clear \n"), Some(InputIntent::Clear));
        assert_eq!(
            parse_input_intent("/clearly"),
            Some(InputIntent::AgentPrompt("/clearly".into()))
        );
    }

    #[test]
    fn command_prompt_format() {
        assert_eq!(agent_prompt_for_command("ls", "a\nb"), "Executed 'ls'. Output:\na\nb");
    }
}
