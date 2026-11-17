use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const BASE: &str = "You are piro, a coding agent working in the user's terminal. Use your tools to inspect \
and change the workspace: read, write and edit for files, bash for everything else (search with rg, grep or \
find; builds, tests, git). Read a file before you edit it. Make focused changes and verify them by running the \
relevant checks. Keep replies concise and say what you changed.";

const INSTRUCTION_FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

pub fn system_prompt(cwd: &Path) -> String {
    let mut prompt = format!(
        "{BASE}\n\n# Environment\nWorking directory: {}\nPlatform: {} {}\nDate (UTC): {}\nGit repository: {}\n",
        cwd.display(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        today(),
        if cwd.ancestors().any(|d| d.join(".git").exists()) {
            "yes"
        } else {
            "no"
        },
    );
    for path in instruction_files(cwd) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            prompt.push_str(&format!("\n# Instructions from {}\n{}\n", path.display(), text.trim()));
        }
    }
    prompt
}

fn instruction_files(cwd: &Path) -> Vec<PathBuf> {
    let global = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".piro").join("AGENTS.md"));
    let mut dirs: Vec<&Path> = cwd.ancestors().collect();
    dirs.reverse();
    let local = dirs.into_iter().filter_map(|dir| {
        INSTRUCTION_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    });
    global.filter(|path| path.is_file()).into_iter().chain(local).collect()
}

fn today() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or_default());
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's days-to-civil algorithm (https://howardhinnant.github.io/date_algorithms.html).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_days_to_civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_773), (2026, 11, 16));
    }

    #[test]
    fn nearest_instruction_files_come_last_and_agents_md_wins() {
        let root = std::env::temp_dir().join(format!("piro-prompt-{}", std::process::id()));
        let inner = root.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(root.join("CLAUDE.md"), "outer rules").unwrap();
        std::fs::write(inner.join("AGENTS.md"), "inner rules").unwrap();
        std::fs::write(inner.join("CLAUDE.md"), "ignored").unwrap();
        let prompt = system_prompt(&inner);
        let (outer, inner_at) = (prompt.find("outer rules").unwrap(), prompt.find("inner rules").unwrap());
        assert!(outer < inner_at);
        assert!(!prompt.contains("ignored"));
        assert!(prompt.contains(&format!("Working directory: {}", inner.display())));
        std::fs::remove_dir_all(root).unwrap();
    }
}
