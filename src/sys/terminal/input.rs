//! Converts crossterm events into the transport's events.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as TermButton, MouseEvent, MouseEventKind,
};
use nsh_platform_tui::{Input, Modifiers, MouseButton, MouseInput, MouseKind, RunEvent};

/// A terminal event as the NSH surface takes it. A resize only asks for a redraw.
pub fn convert(event: Event) -> RunEvent {
    match event {
        Event::Resize(..) => RunEvent::Redraw,
        Event::FocusGained => RunEvent::Input(Input::Focus(true)),
        Event::FocusLost => RunEvent::Input(Input::Focus(false)),
        Event::Paste(text) => RunEvent::Input(Input::Paste(text)),
        Event::Mouse(mouse) => RunEvent::Input(Input::Mouse(mouse_input(mouse))),
        Event::Key(key) => key_input(key),
    }
}

fn modifiers(keys: KeyModifiers) -> Modifiers {
    Modifiers {
        ctrl: keys.contains(KeyModifiers::CONTROL),
        alt: keys.contains(KeyModifiers::ALT),
        shift: keys.contains(KeyModifiers::SHIFT),
        cmd: keys.contains(KeyModifiers::SUPER),
    }
}

fn key_input(key: KeyEvent) -> RunEvent {
    let name = key_name(key.code);
    let mut modifiers = modifiers(key.modifiers);
    if key.code == KeyCode::BackTab {
        modifiers.shift = true;
    }
    match key.kind {
        KeyEventKind::Repeat => RunEvent::Input(Input::KeyRepeat { key: name, modifiers }),
        kind => RunEvent::Input(Input::Key {
            key: name,
            modifiers,
            release: kind == KeyEventKind::Release,
        }),
    }
}

/// The key's name in NSH's keymap: a character as itself, the others as their NSH names.
fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Esc => "Escape".into(),
        KeyCode::BackTab => "Tab".into(),
        KeyCode::F(number) => format!("F{number}"),
        other => format!("{other:?}"),
    }
}

fn mouse_input(mouse: MouseEvent) -> MouseInput {
    let mut input = MouseInput::default();
    input.kind = mouse_kind(mouse.kind);
    input.column = mouse.column;
    input.row = mouse.row;
    input.modifiers = modifiers(mouse.modifiers);
    input
}

fn mouse_kind(kind: MouseEventKind) -> MouseKind {
    match kind {
        MouseEventKind::Down(button) => MouseKind::Down(button_of(button)),
        MouseEventKind::Up(button) => MouseKind::Up(button_of(button)),
        MouseEventKind::Moved | MouseEventKind::Drag(_) => MouseKind::Move,
        MouseEventKind::ScrollUp => MouseKind::Wheel { x: 0, y: -1 },
        MouseEventKind::ScrollDown => MouseKind::Wheel { x: 0, y: 1 },
        MouseEventKind::ScrollLeft => MouseKind::Wheel { x: -1, y: 0 },
        MouseEventKind::ScrollRight => MouseKind::Wheel { x: 1, y: 0 },
    }
}

fn button_of(button: TermButton) -> MouseButton {
    match button {
        TermButton::Left => MouseButton::Left,
        TermButton::Right => MouseButton::Right,
        TermButton::Middle => MouseButton::Middle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, held: KeyModifiers) -> RunEvent {
        convert(Event::Key(KeyEvent::new(code, held)))
    }

    #[test]
    fn keys_keep_their_names_and_modifiers() {
        let none = KeyModifiers::NONE;
        let cases = [
            (KeyCode::Enter, none, "Enter", (false, false, false)),
            (KeyCode::Enter, KeyModifiers::SHIFT, "Enter", (false, false, true)),
            (KeyCode::Char('j'), KeyModifiers::CONTROL, "j", (true, false, false)),
            (KeyCode::Esc, none, "Escape", (false, false, false)),
            (KeyCode::Char('c'), KeyModifiers::CONTROL, "c", (true, false, false)),
            (KeyCode::Up, none, "Up", (false, false, false)),
            (KeyCode::Left, none, "Left", (false, false, false)),
            (KeyCode::PageUp, none, "PageUp", (false, false, false)),
            (KeyCode::Char('x'), KeyModifiers::ALT, "x", (false, true, false)),
            (KeyCode::BackTab, none, "Tab", (false, false, true)),
            (KeyCode::F(5), none, "F5", (false, false, false)),
        ];
        for (code, held, name, (ctrl, alt, shift)) in cases {
            let RunEvent::Input(Input::Key {
                key,
                modifiers,
                release,
            }) = press(code, held)
            else {
                panic!("{code:?} is a key press");
            };
            assert_eq!(key, name, "{code:?}");
            assert_eq!(
                modifiers,
                Modifiers {
                    ctrl,
                    alt,
                    shift,
                    cmd: false
                },
                "{code:?}"
            );
            assert!(!release);
        }
    }

    #[test]
    fn releases_and_repeats_are_distinct() {
        let mut press = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
        press.kind = KeyEventKind::Release;
        assert!(matches!(
            convert(Event::Key(press)),
            RunEvent::Input(Input::Key { release: true, .. })
        ));
        press.kind = KeyEventKind::Repeat;
        assert!(matches!(
            convert(Event::Key(press)),
            RunEvent::Input(Input::KeyRepeat { .. })
        ));
    }

    #[test]
    fn mouse_wheel_keeps_position_and_modifiers() {
        let wheel = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 17,
            row: 9,
            modifiers: KeyModifiers::SHIFT,
        });
        let RunEvent::Input(Input::Mouse(mouse)) = convert(wheel) else {
            panic!("wheel is mouse input");
        };
        assert_eq!(mouse.kind, MouseKind::Wheel { x: 0, y: 1 });
        assert_eq!((mouse.column, mouse.row), (17, 9));
        assert!(mouse.modifiers.shift);
    }

    #[test]
    fn mouse_buttons_and_drags() {
        let at = |kind| {
            convert(Event::Mouse(MouseEvent {
                kind,
                column: 2,
                row: 3,
                modifiers: KeyModifiers::NONE,
            }))
        };
        let RunEvent::Input(Input::Mouse(down)) = at(MouseEventKind::Down(TermButton::Right)) else {
            panic!("press is mouse input");
        };
        assert_eq!(down.kind, MouseKind::Down(MouseButton::Right));
        let RunEvent::Input(Input::Mouse(drag)) = at(MouseEventKind::Drag(TermButton::Left)) else {
            panic!("drag is mouse input");
        };
        assert_eq!(drag.kind, MouseKind::Move);
    }

    #[test]
    fn resize_is_a_redraw_and_focus_and_paste_are_input() {
        assert!(matches!(convert(Event::Resize(80, 24)), RunEvent::Redraw));
        assert!(matches!(
            convert(Event::FocusGained),
            RunEvent::Input(Input::Focus(true))
        ));
        assert!(matches!(
            convert(Event::Paste("a\nb".into())),
            RunEvent::Input(Input::Paste(text)) if text == "a\nb"
        ));
    }
}
