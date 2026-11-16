#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CliArgs {
    pub model: Option<String>,
    pub command: Option<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Login { provider: String },
}

pub fn print_help() -> &'static str {
    "Usage: piro [OPTIONS]\n       piro login [PROVIDER]\n\nCommands:\n  login [PROVIDER]              Sign in and store credentials in ~/.piro/auth.json (default: claude)\n\nOptions:\n  -m, --model <PROVIDER/MODEL>  Model in format <provider>/<model> (e.g. ollama/ornith-1.5:35b, openai/gpt-4o, claude/claude-sonnet-4-6, anthropic/claude-sonnet-4-6)\n  -h, --help                    Print help\n"
}

fn parse_model_arg(arg: &str, iter: &mut impl Iterator<Item = String>) -> Result<String, String> {
    if arg == "-m" || arg == "--model" {
        iter.next()
            .ok_or_else(|| "Error: --model requires a <provider>/<model> argument".into())
    } else if let Some(val) = arg.strip_prefix("--model=") {
        Ok(val.to_string())
    } else if let Some(val) = arg.strip_prefix("-m=") {
        Ok(val.to_string())
    } else {
        Err(format!("Unknown argument '{}'. Use --help for usage.", arg))
    }
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<CliArgs, String> {
    let mut iter = args.into_iter().skip(1).peekable();
    if iter.peek().is_some_and(|arg| arg == "login") {
        iter.next();
        return parse_login(iter);
    }
    let mut model = None;
    while let Some(arg) = iter.next() {
        if arg == "-h" || arg == "--help" {
            return Err(print_help().to_string());
        }
        let val = parse_model_arg(&arg, &mut iter)?;
        model = Some(val);
    }
    Ok(CliArgs { model, command: None })
}

fn parse_login(rest: impl Iterator<Item = String>) -> Result<CliArgs, String> {
    let mut provider = None;
    for arg in rest {
        if arg == "-h" || arg == "--help" {
            return Err(print_help().to_string());
        }
        if arg.starts_with('-') || provider.is_some() {
            return Err(format!("Unexpected argument '{arg}'. Usage: piro login [provider]"));
        }
        provider = Some(arg);
    }
    let provider = provider.unwrap_or_else(|| "claude".to_string());
    Ok(CliArgs {
        model: None,
        command: Some(Command::Login { provider }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_model() {
        let args = vec!["piro".into(), "--model".into(), "ollama/ornith-1.5:35b".into()];
        let parsed = parse_args(args).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("ollama/ornith-1.5:35b"));

        let args_short = vec!["piro".into(), "-m".into(), "openai/gpt-4o".into()];
        let parsed_short = parse_args(args_short).unwrap();
        assert_eq!(parsed_short.model.as_deref(), Some("openai/gpt-4o"));

        let args_eq = vec!["piro".into(), "--model=ollama/qwen".into()];
        let parsed_eq = parse_args(args_eq).unwrap();
        assert_eq!(parsed_eq.model.as_deref(), Some("ollama/qwen"));
    }

    #[test]
    fn test_parse_args_help_and_unknown() {
        let help = vec!["piro".into(), "--help".into()];
        assert!(parse_args(help).is_err());

        let unknown = vec!["piro".into(), "--unknown".into()];
        assert!(parse_args(unknown).is_err());

        let missing = vec!["piro".into(), "--model".into()];
        assert!(parse_args(missing).is_err());
    }

    #[test]
    fn test_parse_login_defaults_and_explicit_provider() {
        let default = parse_args(vec!["piro".into(), "login".into()]).unwrap();
        assert_eq!(
            default.command,
            Some(Command::Login {
                provider: "claude".into()
            })
        );
        assert_eq!(default.model, None);

        let explicit = parse_args(vec!["piro".into(), "login".into(), "claude".into()]).unwrap();
        assert_eq!(
            explicit.command,
            Some(Command::Login {
                provider: "claude".into()
            })
        );

        assert!(parse_args(vec!["piro".into(), "login".into(), "a".into(), "b".into()]).is_err());
        assert!(parse_args(vec!["piro".into(), "login".into(), "--bogus".into()]).is_err());
        assert!(parse_args(vec!["piro".into(), "login".into(), "--help".into()]).is_err());
    }
}
