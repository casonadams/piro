#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CliArgs {
    pub model: Option<String>,
}

pub fn print_help() -> &'static str {
    "Usage: piro [OPTIONS]\n\nOptions:\n  -m, --model <PROVIDER/MODEL>  Model in format <provider>/<model> (e.g. ollama/ornith-1.5:35b, openai/gpt-4o)\n  -h, --help                    Print help\n"
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
    let mut model = None;
    let mut iter = args.into_iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "-h" || arg == "--help" {
            return Err(print_help().to_string());
        }
        let val = parse_model_arg(&arg, &mut iter)?;
        model = Some(val);
    }
    Ok(CliArgs { model })
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
}
