//! `piro login [provider]`: the blocking terminal flow.

use std::io::{self, BufRead, Write};

use crate::auth::claude;

pub fn run_login(provider: &str) -> Result<(), String> {
    match provider {
        "claude" | "claude-code" | "claude-oauth" => crate::sys::runtime().block_on(login_claude()),
        other => Err(format!("Unknown provider '{other}'. Supported providers: 'claude'")),
    }
}

async fn login_claude() -> Result<(), String> {
    if claude::has_local_credentials().await && prompt_yes_no("Import existing Claude Code credentials? [Y/n] ")? {
        if !claude::import_local_credentials().await? {
            return Err("Claude Code credentials disappeared before they could be imported".into());
        }
        println!("Imported existing Claude Code credentials.");
        return Ok(());
    }
    let request = claude::begin_login();
    println!("Sign in to Claude by opening this URL:\n\n{}\n", request.url);
    if let Err(e) = claude::open_browser(&request.url) {
        println!("(Could not open a browser: {e})");
    }
    let pasted = prompt("Paste the authorization code (or CODE#STATE): ")?;
    match claude::complete_login(&request, &pasted).await? {
        Some(email) => println!("Logged in to Claude as {email}."),
        None => println!("Logged in to Claude."),
    }
    Ok(())
}

fn prompt(message: &str) -> Result<String, String> {
    print!("{message}");
    io::stdout()
        .flush()
        .map_err(|e| format!("Failed to write prompt: {e}"))?;
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read input: {e}"))?;
    Ok(line.trim().to_string())
}

fn prompt_yes_no(message: &str) -> Result<bool, String> {
    Ok(!prompt(message)?.to_ascii_lowercase().starts_with('n'))
}
