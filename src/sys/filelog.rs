//! Logging through the `log` facade into one file: the TUI owns stdout and stderr.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

struct FileLogger(Mutex<Option<File>>);

static LOGGER: FileLogger = FileLogger(Mutex::new(None));

impl log::Log for FileLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let mut file = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = file.as_mut() {
            let _ = writeln!(file, "{} {}: {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

/// Appends records at info and above to `path`.
pub fn init(path: &Path) -> io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    *LOGGER.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
    log::set_logger(&LOGGER).map_err(|_| io::Error::other("a logger is already set"))?;
    log::set_max_level(log::LevelFilter::Info);
    Ok(())
}
