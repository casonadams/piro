use std::borrow::Cow;

const TAB_WIDTH: usize = 4;

/// Text a terminal cell can hold: ANSI escape sequences and control characters removed,
/// tabs expanded, and a bare `\r` restarting its line as a terminal would.
/// ratatui panics on control characters in cells, so every displayed string passes through here.
pub fn clean_text(text: &str) -> Cow<'_, str> {
    if !text.chars().any(needs_cleaning) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut line_start = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                out.push('\n');
                line_start = out.len();
            }
            '\t' => {
                let column = out[line_start..].chars().count();
                out.extend(std::iter::repeat_n(' ', TAB_WIDTH - column % TAB_WIDTH));
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\r' => out.truncate(line_start),
            '\u{1b}' => skip_escape(&mut chars),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

fn needs_cleaning(c: char) -> bool {
    c != '\n' && c.is_control()
}

/// Skips the rest of an escape sequence: CSI up to its final byte, OSC up to BEL or ST, else one character.
fn skip_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        Some('[') => {
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
        Some(']') => {
            while let Some(c) = chars.next() {
                if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                    break;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_plain_text_borrowed() {
        assert!(matches!(clean_text("plain 📅 text\nline"), Cow::Borrowed(_)));
    }

    #[test]
    fn cleans_terminal_output_into_cell_safe_text() {
        let cases = [
            ("\u{1b}[1;34mdir\u{1b}[0m file", "dir file"),
            ("\u{1b}]8;;https://x\u{7}link\u{1b}]8;;\u{1b}\\", "link"),
            ("a\tb\n\tc", "a   b\n    c"),
            ("50%\r100%", "100%"),
            ("crlf\r\nnext", "crlf\nnext"),
            ("bell\u{7}\u{8}\u{7f}", "bell"),
            ("\u{1b}(Bkeep", "Bkeep"),
        ];
        for (input, want) in cases {
            assert_eq!(clean_text(input), want, "{input:?}");
        }
    }

    #[test]
    fn output_has_no_control_characters_but_newlines() {
        let dirty = "\u{1b}[31mred\u{1b}[0m\t\u{0}\u{1}\r\n\u{9b}x";
        assert!(clean_text(dirty).chars().all(|c| c == '\n' || !c.is_control()));
    }
}
