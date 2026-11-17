//! Saved conversations: one JSON Lines file per conversation, a `{"cwd": ...}` header then one message per line.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::AgentMessage;

#[derive(Serialize, Deserialize)]
struct Header {
    cwd: PathBuf,
}

pub struct Transcript {
    dir: Option<PathBuf>,
    cwd: PathBuf,
    file: Option<PathBuf>,
}

impl Transcript {
    pub fn new(dir: Option<PathBuf>, cwd: PathBuf) -> Self {
        Self { dir, cwd, file: None }
    }

    pub fn append(&mut self, msg: &AgentMessage) {
        if let Err(e) = self.try_append(msg) {
            log::warn!("cannot save the conversation: {e}");
        }
    }

    pub fn restart(&mut self) {
        self.file = None;
    }

    pub fn rewrite(&mut self, history: &[AgentMessage]) {
        self.restart();
        history.iter().for_each(|msg| self.append(msg));
    }

    pub fn resume_latest(&mut self) -> Vec<AgentMessage> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .collect();
        files.sort_unstable_by(|a, b| b.cmp(a));
        for path in files {
            if let Some(messages) = read_conversation(&path, &self.cwd) {
                self.file = Some(path);
                return messages;
            }
        }
        Vec::new()
    }

    fn try_append(&mut self, msg: &AgentMessage) -> std::io::Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let path = match &self.file {
            Some(path) => path.clone(),
            None => self.create(dir.clone())?,
        };
        let mut file = OpenOptions::new().append(true).open(&path)?;
        writeln!(file, "{}", serde_json::to_string(msg)?)
    }

    fn create(&mut self, dir: PathBuf) -> std::io::Result<PathBuf> {
        static SEQUENCE: AtomicU32 = AtomicU32::new(0);
        fs::create_dir_all(&dir)?;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{millis:013}-{seq:06}-{}.jsonl", std::process::id()));
        let header = Header { cwd: self.cwd.clone() };
        fs::write(&path, format!("{}\n", serde_json::to_string(&header)?))?;
        self.file = Some(path.clone());
        Ok(path)
    }
}

fn read_conversation(path: &Path, cwd: &Path) -> Option<Vec<AgentMessage>> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let header: Header = serde_json::from_str(lines.next()?).ok()?;
    (header.cwd == cwd).then(|| lines.filter_map(|line| serde_json::from_str(line).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumes_the_latest_conversation_of_the_same_directory() {
        let dir = std::env::temp_dir().join(format!("piro-transcript-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut other = Transcript::new(Some(dir.clone()), "/elsewhere".into());
        let mut here = Transcript::new(Some(dir.clone()), "/here".into());
        here.append(&AgentMessage::User("old".into()));
        here.rewrite(&[AgentMessage::User("summary".into())]);
        std::thread::sleep(std::time::Duration::from_millis(2));
        other.append(&AgentMessage::User("not mine".into()));

        let mut resumed = Transcript::new(Some(dir.clone()), "/here".into());
        assert_eq!(resumed.resume_latest(), [AgentMessage::User("summary".into())]);
        resumed.append(&AgentMessage::User("next".into()));
        let mut again = Transcript::new(Some(dir.clone()), "/here".into());
        assert_eq!(again.resume_latest().len(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
