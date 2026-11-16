use std::io::Write;
use std::path::PathBuf;

pub const HISTORY_CAP: usize = 1000;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct History {
    entries: Vec<String>,
    cursor: Option<usize>,
    draft: String,
}

impl History {
    /// Lines of a history file: trimmed, blank lines and consecutive repeats dropped, capped.
    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let mut history = Self::default();
        for line in lines.into_iter().map(str::trim).filter(|l| !l.is_empty()) {
            history.record(line);
        }
        history
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Records a submitted input and ends any recall. Returns whether it was new, so the caller can persist it.
    pub fn record(&mut self, entry: &str) -> bool {
        self.cursor = None;
        self.draft.clear();
        if self.entries.last().map(String::as_str) == Some(entry) {
            return false;
        }
        self.push_entry(entry);
        true
    }

    /// Steps back to the previous entry. The first step keeps `current` as the draft.
    pub fn older(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let index = match self.cursor {
            None => {
                self.draft = current.to_owned();
                self.entries.len() - 1
            }
            Some(i) => i.saturating_sub(1),
        };
        self.cursor = Some(index);
        Some(self.entries[index].clone())
    }

    /// Steps forward; past the newest entry it returns the draft and ends the recall.
    pub fn newer(&mut self) -> Option<String> {
        let index = self.cursor?;
        if index + 1 < self.entries.len() {
            self.cursor = Some(index + 1);
            Some(self.entries[index + 1].clone())
        } else {
            self.cursor = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    fn push_entry(&mut self, entry: &str) {
        if self.entries.len() >= HISTORY_CAP {
            self.entries.remove(0);
        }
        self.entries.push(entry.to_owned());
    }
}

/// `$HOME/.piro/history`, creating its directory.
pub fn history_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = PathBuf::from(home).join(".piro");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("history"))
}

pub fn load_history() -> History {
    if cfg!(test) {
        return History::default();
    }
    let contents = history_file().and_then(|p| std::fs::read_to_string(p).ok());
    History::from_lines(contents.as_deref().unwrap_or_default().lines())
}

/// Appends a submitted input to the history file. Tests never write the user's file.
pub fn persist_entry(entry: &str) {
    if cfg!(test) {
        return;
    }
    let Some(path) = history_file() else { return };
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{entry}");
    }
}

/// Whether the caret (a UTF-16 offset, as NSH reports it) is on the first line of `text`.
pub fn caret_on_first_line(text: &str, caret: u32) -> bool {
    !text.encode_utf16().take(caret as usize).any(|u| u == u16::from(b'\n'))
}

/// Whether the caret (a UTF-16 offset, as NSH reports it) is on the last line of `text`.
pub fn caret_on_last_line(text: &str, caret: u32) -> bool {
    !text.encode_utf16().skip(caret as usize).any(|u| u == u16::from(b'\n'))
}

/// The byte index of a UTF-16 offset in `text`, clamped to the end.
pub fn byte_index(text: &str, utf16: u32) -> usize {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        if units >= utf16 as usize {
            return index;
        }
        units += c.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_dedupes_consecutive_and_reports_new_entries() {
        let mut h = History::default();
        assert!(h.record("ls"));
        assert!(!h.record("ls"));
        assert!(h.record("pwd"));
        assert!(h.record("ls"));
        assert_eq!(h.entries(), ["ls", "pwd", "ls"]);
    }

    #[test]
    fn from_lines_trims_skips_blank_and_dedupes() {
        let h = History::from_lines(["  a ", "", "a", "b"]);
        assert_eq!(h.entries(), ["a", "b"]);
    }

    #[test]
    fn cap_drops_oldest() {
        let lines: Vec<String> = (0..HISTORY_CAP + 5).map(|i| format!("cmd {i}")).collect();
        let h = History::from_lines(lines.iter().map(String::as_str));
        assert_eq!(h.entries().len(), HISTORY_CAP);
        assert_eq!(h.entries()[0], "cmd 5");
        assert_eq!(h.entries().last().map(String::as_str), Some("cmd 1004"));
    }

    #[test]
    fn older_and_newer_restore_draft() {
        let mut h = History::from_lines(["one", "two"]);
        assert_eq!(h.older("draft").as_deref(), Some("two"));
        assert_eq!(h.older("two").as_deref(), Some("one"));
        assert_eq!(h.older("one").as_deref(), Some("one"));
        assert_eq!(h.newer().as_deref(), Some("two"));
        assert_eq!(h.newer().as_deref(), Some("draft"));
        assert_eq!(h.newer(), None);
    }

    #[test]
    fn older_on_empty_history_is_none() {
        let mut h = History::default();
        assert_eq!(h.older("draft"), None);
    }

    #[test]
    fn record_ends_recall() {
        let mut h = History::from_lines(["one"]);
        h.older("typed");
        h.record("one");
        assert_eq!(h.newer(), None);
        assert_eq!(h.older("").as_deref(), Some("one"));
    }

    #[test]
    fn caret_lines_use_utf16_offsets() {
        let text = "a😀\nbc\nd";
        // "a😀" is three UTF-16 units, so the first newline is at offset 3.
        assert!(caret_on_first_line(text, 0));
        assert!(caret_on_first_line(text, 3));
        assert!(!caret_on_first_line(text, 4));
        assert!(!caret_on_last_line(text, 3));
        assert!(caret_on_last_line(text, 7));
        assert!(caret_on_last_line("single", 2));
    }

    #[test]
    fn byte_index_maps_utf16_offsets() {
        let text = "a😀b";
        assert_eq!(byte_index(text, 0), 0);
        assert_eq!(byte_index(text, 1), 1);
        assert_eq!(byte_index(text, 3), 5);
        assert_eq!(byte_index(text, 4), text.len());
        assert_eq!(byte_index(text, 99), text.len());
    }
}
