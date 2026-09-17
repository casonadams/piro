use ansi_to_tui::IntoText;
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

pub fn str_width(s: &str) -> usize {
    s.chars().map(|c| UnicodeWidthChar::width(c).unwrap_or(0)).sum()
}

pub fn pad_spaces(s: &str, target_w: usize) -> String {
    let w = str_width(s);
    if w < target_w {
        format!("{}{}", s, " ".repeat(target_w - w))
    } else {
        s.to_string()
    }
}

struct WrapState {
    target_w: usize,
    cur: Vec<Span<'static>>,
    cur_w: usize,
    out: Vec<Vec<Span<'static>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLine {
    pub spans: Vec<Span<'static>>,
    pub is_continuation: bool,
}

fn push_word(word: &str, style: ratatui::style::Style, st: &mut WrapState) {
    let ww = str_width(word);
    if st.cur_w + ww <= st.target_w {
        st.cur.push(Span::styled(word.to_string(), style));
        st.cur_w += ww;
        return;
    }
    if st.cur_w > 0 {
        st.cur.push(Span::raw(" ".repeat(st.target_w - st.cur_w)));
        st.out.push(std::mem::take(&mut st.cur));
        st.cur_w = 0;
    }
    let trimmed = word.trim_start();
    let tw = str_width(trimmed);
    if tw <= st.target_w {
        st.cur.push(Span::styled(trimmed.to_string(), style));
        st.cur_w = tw;
    } else {
        for c in trimmed.chars() {
            let cw = UnicodeWidthChar::width(c).unwrap_or(0);
            if st.cur_w + cw > st.target_w {
                st.out.push(std::mem::take(&mut st.cur));
                st.cur_w = 0;
            }
            st.cur.push(Span::styled(c.to_string(), style));
            st.cur_w += cw;
        }
    }
}

fn wrap_spans(spans: Vec<Span<'static>>, target_w: usize, out: &mut Vec<WrappedLine>) {
    let total: usize = spans.iter().map(|s| str_width(&s.content)).sum();
    if total <= target_w {
        let mut line = spans;
        if total < target_w {
            line.push(Span::raw(" ".repeat(target_w - total)));
        }
        out.push(WrappedLine {
            spans: line,
            is_continuation: false,
        });
        return;
    }

    let mut st = WrapState {
        target_w,
        cur: Vec::new(),
        cur_w: 0,
        out: Vec::new(),
    };

    for span in spans {
        for word in span.content.split_inclusive(char::is_whitespace) {
            push_word(word, span.style, &mut st);
        }
    }
    if st.cur_w > 0 {
        st.cur.push(Span::raw(" ".repeat(target_w - st.cur_w)));
        st.out.push(st.cur);
    }
    for (idx, line) in st.out.into_iter().enumerate() {
        out.push(WrappedLine {
            spans: line,
            is_continuation: idx > 0,
        });
    }
}

pub fn wrap_ansi(raw: &str, target_w: usize) -> Vec<WrappedLine> {
    let text = match raw.as_bytes().into_text() {
        Ok(t) => t,
        Err(_) => {
            return vec![WrappedLine {
                spans: vec![Span::raw(pad_spaces(raw, target_w))],
                is_continuation: false,
            }];
        }
    };

    let mut out = Vec::new();
    for l in text.lines {
        let spans: Vec<Span<'static>> = l
            .spans
            .into_iter()
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect();
        wrap_spans(spans, target_w, &mut out);
    }
    if out.is_empty() {
        out.push(WrappedLine {
            spans: vec![Span::raw(" ".repeat(target_w))],
            is_continuation: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_wrapping() {
        let raw = "The quick brown fox jumps over the lazy dog";
        let wrapped = wrap_ansi(raw, 16);
        assert!(wrapped.len() > 1);
        for line in &wrapped {
            let line_w: usize = line.spans.iter().map(|s| str_width(&s.content)).sum();
            assert_eq!(line_w, 16);
        }
    }

    #[test]
    fn test_ansi_parsing_and_width() {
        let raw = "\x1b[32mhello\x1b[0m \x1b[31mworld\x1b[0m";
        let wrapped = wrap_ansi(raw, 20);
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0].spans[0].content, "hello");
        assert_eq!(wrapped[0].spans[1].content, " ");
        assert_eq!(wrapped[0].spans[2].content, "world");
    }

    #[test]
    fn test_multiline_preserves_line_breaks() {
        let raw = "line 1\nline 2";
        let wrapped = wrap_ansi(raw, 20);
        assert_eq!(wrapped.len(), 2);
        assert!(!wrapped[0].is_continuation);
        assert!(!wrapped[1].is_continuation);
        assert_eq!(wrapped[0].spans[0].content, "line 1");
        assert_eq!(wrapped[1].spans[0].content, "line 2");
    }
}
