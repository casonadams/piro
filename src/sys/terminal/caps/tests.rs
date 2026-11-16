use super::*;

fn env(build: impl FnOnce(&mut TermEnv)) -> TermEnv {
    let mut env = TermEnv::default();
    build(&mut env);
    env
}

#[test]
fn replies_carry_colors_keyboard_and_completion() {
    let replies = parse_replies(b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\\x1b]11;rgb:1e1e/2020/3030\x07\x1b[?1u\x1b[?62;22c");
    assert_eq!(replies.foreground, Some([255, 255, 255]));
    assert_eq!(replies.background, Some([0x1e, 0x20, 0x30]));
    assert!(replies.keyboard && replies.complete);
}

#[test]
fn partial_and_short_replies() {
    assert_eq!(parse_color("rgb:f/8/0"), Some([255, 136, 0]));
    assert_eq!(parse_color("rgb:12345/0/0"), None);
    assert_eq!(parse_color("#ffffff"), None);
    assert_eq!(
        parse_replies(b"\x1b[?1;2c"),
        Replies {
            complete: true,
            ..Replies::default()
        }
    );
    // A truncated OSC answer keeps nothing and does not hang the parser.
    assert_eq!(parse_replies(b"\x1b]11;rgb:00"), Replies::default());
    assert_eq!(parse_replies(b"\x1b"), Replies::default());
}

#[test]
fn background_sets_scheme_and_colors() {
    let light = Replies {
        background: Some([0xfd, 0xf6, 0xe3]),
        ..Replies::default()
    };
    let found = capabilities(&light, &env(|e| e.colorterm = "truecolor".into()));
    assert_eq!(found.background, ColorScheme::Light);
    assert_eq!(found.background_color, Some([0xfd, 0xf6, 0xe3]));
    assert_eq!(found.colors, ColorDepth::TrueColor);
    let dark = capabilities(&Replies::default(), &env(|e| e.term = "xterm-256color".into()));
    assert_eq!(dark.background, ColorScheme::Dark);
    assert_eq!(dark.colors, ColorDepth::Ansi256);
    assert!(!dark.hyperlinks);
}

#[test]
fn color_depth_follows_the_overrides() {
    let depth = |e: TermEnv| color_depth(&e);
    assert_eq!(depth(TermEnv::default()), ColorDepth::Ansi16);
    assert_eq!(depth(env(|e| e.term = "xterm".into())), ColorDepth::Ansi16);
    assert_eq!(
        depth(env(|e| {
            e.colorterm = "truecolor".into();
            e.nsh_tui_colors = "16".into();
        })),
        ColorDepth::Ansi16
    );
    assert_eq!(
        depth(env(|e| {
            e.colorterm = "truecolor".into();
            e.no_color = true;
        })),
        ColorDepth::Monochrome
    );
    assert_eq!(
        depth(env(|e| {
            e.no_color = true;
            e.nsh_tui_colors = "256".into();
        })),
        ColorDepth::Ansi256
    );
}

#[test]
fn color_depth_follows_terminal_markers() {
    // tmux and SSH drop COLORTERM; the terminal's own markers remain.
    for setup in [
        |e: &mut TermEnv| {
            e.term_program = "ghostty".into();
            e.term = "xterm-256color".into();
        },
        |e: &mut TermEnv| {
            e.brand_window = true;
            e.term = "tmux-256color".into();
        },
        |e: &mut TermEnv| e.term = "xterm-kitty".into(),
    ] {
        assert_eq!(color_depth(&env(setup)), ColorDepth::TrueColor);
    }
}

#[test]
fn linux_console_has_no_hyperlinks() {
    let caps = capabilities(&Replies::default(), &env(|e| e.term = "linux".into()));
    assert!(!caps.hyperlinks);
}

#[test]
fn keyboard_protocol_only_where_the_answer_holds() {
    let answered = Replies {
        keyboard: true,
        ..Replies::default()
    };
    assert!(keyboard_protocol(&answered, &env(|e| e.term_program = "kitty".into())));
    assert!(!keyboard_protocol(&Replies::default(), &TermEnv::default()));
    for setup in [
        |e: &mut TermEnv| e.term_program = "Apple_Terminal".into(),
        |e: &mut TermEnv| e.term_program = "vscode".into(),
        |e: &mut TermEnv| e.vte = true,
        |e: &mut TermEnv| {
            e.term_program = "tmux".into();
            e.term_program_version = "3.2a".into();
        },
    ] {
        assert!(!keyboard_protocol(&answered, &env(setup)));
    }
    assert!(keyboard_protocol(
        &answered,
        &env(|e| {
            e.term_program = "tmux".into();
            e.term_program_version = "3.4".into();
        })
    ));
}

#[test]
fn hyperlinks_only_where_the_terminal_takes_them() {
    let links = |setup: fn(&mut TermEnv)| capabilities(&Replies::default(), &env(setup)).hyperlinks;
    assert!(links(|e| e.term_program = "WezTerm".into()));
    assert!(links(|e| e.vte = true));
    assert!(!links(|e| e.term = "linux".into()));
    assert!(!links(|e| e.term = "xterm-256color".into()));
}
