use std::process::ExitCode;

use nsh_platform_tui::Root;
use piro::app::cli::{self, CliArgs, Command};
use piro::{launch, provider, sys};

fn main() -> ExitCode {
    let args = match cli::parse_args(std::env::args()) {
        Ok(args) => args,
        Err(text) if text == cli::print_help() => {
            print!("{text}");
            return ExitCode::SUCCESS;
        }
        Err(text) => {
            eprintln!("{text}");
            return ExitCode::FAILURE;
        }
    };
    match start(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(text) => {
            eprintln!("{text}");
            ExitCode::FAILURE
        }
    }
}

fn start(args: CliArgs) -> Result<(), String> {
    if let Some(Command::Login { provider }) = args.command {
        return piro::auth::run_login(&provider);
    }
    if let Some(spec) = &args.model {
        provider::provider_from_spec(spec)?;
    }
    launch::set(launch::LaunchOptions {
        model_spec: args.model,
        resume: args.resume,
        auto_approve: args.auto_approve,
    })?;
    sys::runtime();
    let ui_sets = vec![nsh_base::registry(), piro::registry()];
    sys::host::run(Root::of::<piro::App>(), ui_sets).map_err(|error| error.to_string())
}
