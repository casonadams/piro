//! Terminal capabilities from the startup answers and the environment.

use nsh_core::platform::ColorScheme;
use nsh_platform_tui::{ColorDepth, TerminalCapabilities};

/// Terminals known to draw 24-bit color and OSC 8 links, by `TERM_PROGRAM` or by `TERM`.
/// tmux and SSH drop `COLORTERM`; these keep their own markers.
const KNOWN_PROGRAMS: [&str; 7] = [
    "iterm.app",
    "ghostty",
    "wezterm",
    "vscode",
    "warpterminal",
    "rio",
    "alacritty",
];
const KNOWN_TERMS: [&str; 6] = ["kitty", "alacritty", "ghostty", "foot", "wezterm", "rio"];

/// The environment capability detection reads, captured once at startup.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TermEnv {
    pub term: String,
    pub colorterm: String,
    pub nsh_tui_colors: String,
    pub term_program: String,
    pub term_program_version: String,
    pub no_color: bool,
    pub vte: bool,
    /// A terminal-specific variable such as `KITTY_WINDOW_ID` is set.
    pub brand_window: bool,
}

impl TermEnv {
    pub fn from_process() -> Self {
        const BRAND_VARS: [&str; 5] = [
            "KITTY_WINDOW_ID",
            "ALACRITTY_WINDOW_ID",
            "WEZTERM_EXECUTABLE",
            "GHOSTTY_RESOURCES_DIR",
            "WT_SESSION",
        ];
        let text = |name: &str| std::env::var(name).unwrap_or_default();
        let set = |name: &str| std::env::var(name).is_ok_and(|value| !value.is_empty());
        Self {
            term: text("TERM"),
            colorterm: text("COLORTERM"),
            nsh_tui_colors: text("NSH_TUI_COLORS"),
            term_program: text("TERM_PROGRAM"),
            term_program_version: text("TERM_PROGRAM_VERSION"),
            no_color: set("NO_COLOR"),
            vte: set("VTE_VERSION"),
            brand_window: BRAND_VARS.iter().any(|name| set(name)),
        }
    }
}

/// The terminal's answers to the startup queries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Replies {
    pub foreground: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    /// The kitty keyboard protocol's flags query was answered.
    pub keyboard: bool,
    /// Primary device attributes arrived: no later answer will.
    pub complete: bool,
}

/// Reads OSC 10 / 11 color answers, the kitty flags answer (`CSI ? flags u`) and DA1 (`CSI ? ... c`).
pub fn parse_replies(bytes: &[u8]) -> Replies {
    let mut replies = Replies::default();
    let mut rest = bytes;
    while let Some(at) = rest.iter().position(|&b| b == 0x1b) {
        rest = &rest[at..];
        rest = match rest.get(1) {
            Some(b']') => &rest[2 + parse_osc(&rest[2..], &mut replies)..],
            Some(b'[') => &rest[2 + parse_csi(&rest[2..], &mut replies)..],
            _ => &rest[1..],
        };
    }
    replies
}

/// Parses one OSC body, returning the bytes it spans including its terminator.
fn parse_osc(body: &[u8], replies: &mut Replies) -> usize {
    let end = body.iter().position(|&b| b == 0x07 || b == 0x1b).unwrap_or(body.len());
    let text = String::from_utf8_lossy(&body[..end]);
    if let Some((code, value)) = text.split_once(';') {
        let color = parse_color(value);
        match code {
            "10" => replies.foreground = color.or(replies.foreground),
            "11" => replies.background = color.or(replies.background),
            _ => {}
        }
    }
    (end + 1).min(body.len())
}

/// Parses one CSI body, returning the bytes it spans including its final byte.
fn parse_csi(body: &[u8], replies: &mut Replies) -> usize {
    let Some(end) = body.iter().position(|b| (0x40..=0x7e).contains(b)) else {
        return body.len();
    };
    if body.first() == Some(&b'?') {
        match body[end] {
            b'u' => replies.keyboard = true,
            b'c' => replies.complete = true,
            _ => {}
        }
    }
    end + 1
}

/// An X11 color as terminals answer it: `rgb:` (or `rgba:`) with one to four hex digits per channel.
fn parse_color(value: &str) -> Option<[u8; 3]> {
    let channels = value.strip_prefix("rgb:").or_else(|| value.strip_prefix("rgba:"))?;
    let mut out = [0; 3];
    let mut parts = channels.split('/');
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 4 {
            return None;
        }
        let value = u32::from_str_radix(part, 16).ok()?;
        let max = (1u32 << (4 * part.len())) - 1;
        *slot = ((value * 255 + max / 2) / max) as u8;
    }
    Some(out)
}

/// The capabilities the startup answers and the environment give.
pub fn capabilities(replies: &Replies, env: &TermEnv) -> TerminalCapabilities {
    let mut capabilities = TerminalCapabilities::default();
    capabilities.foreground_color = replies.foreground;
    capabilities.background_color = replies.background;
    if let Some(background) = replies.background {
        capabilities.background = scheme(background);
    }
    capabilities.hyperlinks = hyperlinks(env);
    capabilities.colors = color_depth(env);
    capabilities
}

/// `NSH_TUI_COLORS` wins, then `NO_COLOR` (no colors at all), then the terminal's
/// advertised depth, then `TERM`.
fn color_depth(env: &TermEnv) -> ColorDepth {
    match env.nsh_tui_colors.to_ascii_lowercase().as_str() {
        "truecolor" | "24bit" => ColorDepth::TrueColor,
        "256" => ColorDepth::Ansi256,
        "16" => ColorDepth::Ansi16,
        "mono" | "none" => ColorDepth::Monochrome,
        _ if env.no_color => ColorDepth::Monochrome,
        _ if truecolor(env) => ColorDepth::TrueColor,
        _ if env.term.contains("256color") => ColorDepth::Ansi256,
        _ => ColorDepth::Ansi16,
    }
}

fn truecolor(env: &TermEnv) -> bool {
    let colorterm = env.colorterm.to_ascii_lowercase();
    colorterm == "truecolor" || colorterm == "24bit" || known_terminal(env)
}

/// OSC 8 links are taken by known terminals and VTE; anything else (the Linux console, an
/// unknown `xterm`) gets plain text, as links the terminal cannot draw would show raw sequences.
fn hyperlinks(env: &TermEnv) -> bool {
    env.vte || known_terminal(env)
}

fn known_terminal(env: &TermEnv) -> bool {
    let term = env.term.to_ascii_lowercase();
    let program = env.term_program.to_ascii_lowercase();
    env.brand_window
        || KNOWN_PROGRAMS.contains(&program.as_str())
        || KNOWN_TERMS.iter().any(|brand| term.contains(brand))
}

/// Light or dark by the background's relative luminance (WCAG).
fn scheme([r, g, b]: [u8; 3]) -> ColorScheme {
    let linear = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    if luminance > 0.4 {
        ColorScheme::Light
    } else {
        ColorScheme::Dark
    }
}

/// Whether to push the kitty keyboard flags: the terminal answered their query and is not one
/// whose answer overstates what it does (Apple's Terminal, VS Code, VTE) or a tmux before 3.3.
pub fn keyboard_protocol(replies: &Replies, env: &TermEnv) -> bool {
    if !replies.keyboard || env.vte {
        return false;
    }
    match env.term_program.as_str() {
        "Apple_Terminal" | "vscode" => false,
        "tmux" => tmux_at_least(&env.term_program_version, (3, 3)),
        _ => true,
    }
}

fn tmux_at_least(version: &str, minimum: (u32, u32)) -> bool {
    let mut parts = version
        .split(|c: char| !c.is_ascii_digit())
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let found = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    found >= minimum
}

#[cfg(test)]
mod tests;
