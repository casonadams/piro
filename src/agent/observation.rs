use crate::agent::tool::ToolResult;

const MAX_LINES: usize = 200;
const HEAD_LINES: usize = 120;
const TAIL_LINES: usize = 79;
const MAX_BYTES: usize = 40_000;

pub fn observation(result: &ToolResult) -> String {
    let text = cap_bytes(truncate_lines(&result.output));
    if text.trim().is_empty() {
        "(no output)".into()
    } else {
        text
    }
}

fn truncate_lines(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= MAX_LINES {
        return output.to_string();
    }
    let omitted = lines.len() - HEAD_LINES - TAIL_LINES;
    let notice = format!("... [output truncated: {omitted} lines omitted] ...");
    let mut kept = Vec::with_capacity(MAX_LINES);
    kept.extend_from_slice(&lines[..HEAD_LINES]);
    kept.push(&notice);
    kept.extend_from_slice(&lines[lines.len() - TAIL_LINES..]);
    kept.join("\n")
}

fn cap_bytes(text: String) -> String {
    if text.len() <= MAX_BYTES {
        return text;
    }
    let head = text.floor_char_boundary(MAX_BYTES / 2);
    let tail = text.ceil_char_boundary(text.len() - MAX_BYTES / 2);
    format!(
        "{}\n... [output truncated: {} bytes omitted] ...\n{}",
        &text[..head],
        tail - head,
        &text[tail..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_output_keeps_head_and_tail_lines() {
        let long = (0..300).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let text = observation(&ToolResult::ok(long));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 200);
        assert_eq!(lines[119], "line 119");
        assert_eq!(lines[120], "... [output truncated: 101 lines omitted] ...");
        assert_eq!(lines[199], "line 299");
    }

    #[test]
    fn one_huge_line_is_capped_by_bytes() {
        let text = observation(&ToolResult::ok("é".repeat(100_000)));
        assert!(text.len() < MAX_BYTES + 100);
        assert!(text.contains("bytes omitted"));
        assert_eq!(observation(&ToolResult::ok("")), "(no output)");
    }
}
