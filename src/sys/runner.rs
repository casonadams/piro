use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::sys::sanitize::clean_text;

#[derive(Debug)]
pub enum RunnerMsg {
    Line(String),
    Done(Option<i32>, Duration),
}

pub struct ProcessHandle {
    supervisor: AbortHandle,
    out_abort: AbortHandle,
    err_abort: AbortHandle,
    #[cfg(unix)]
    pid: Option<u32>,
}

impl ProcessHandle {
    #[cfg(test)]
    pub fn mock(abort: AbortHandle) -> Self {
        Self {
            supervisor: abort.clone(),
            out_abort: abort.clone(),
            err_abort: abort,
            #[cfg(unix)]
            pid: None,
        }
    }

    pub fn kill(&mut self) {
        self.out_abort.abort();
        self.err_abort.abort();

        #[cfg(unix)]
        if let Some(pid) = self.pid {
            unsafe {
                let p = pid as i32;
                let _ = libc::kill(-p, libc::SIGTERM);
                let _ = libc::kill(-p, libc::SIGKILL);
                let _ = libc::kill(p, libc::SIGTERM);
                let _ = libc::kill(p, libc::SIGKILL);
            }
        }

        self.supervisor.abort();
    }
}

fn spawn_stream<R: AsyncRead + Unpin + Send + 'static>(
    stream: Option<R>,
    tx: mpsc::UnboundedSender<RunnerMsg>,
) -> (tokio::task::JoinHandle<()>, AbortHandle) {
    let task = tokio::spawn(async move {
        let Some(s) = stream else { return };
        let mut reader = BufReader::new(s);
        let mut buf = Vec::new();
        while reader.read_until(b'\n', &mut buf).await.is_ok_and(|n| n > 0) {
            let raw = String::from_utf8_lossy(&buf);
            let line = clean_text(raw.trim_end_matches(['\n', '\r'])).into_owned();
            buf.clear();
            if tx.send(RunnerMsg::Line(line)).is_err() {
                break;
            }
        }
    });
    let abort = task.abort_handle();
    (task, abort)
}

/// A script for a shell. `shell: None` is the user's `$SHELL`; `cwd: None` is piro's own directory.
pub struct ShellJob {
    pub script: String,
    pub shell: Option<&'static str>,
    pub cwd: Option<PathBuf>,
}

impl ShellJob {
    pub fn user(script: String) -> Self {
        Self {
            script,
            shell: None,
            cwd: None,
        }
    }
}

fn build_shell_command(job: &ShellJob) -> Command {
    let shell = match job.shell {
        Some(shell) => shell.to_owned(),
        None => std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
    };
    let mut cmd = Command::new(shell);
    cmd.arg("-c")
        .arg(&job.script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("NO_COLOR", "1")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat");
    if let Some(cwd) = &job.cwd {
        cmd.current_dir(cwd);
    }

    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

pub fn spawn_command(job: ShellJob, tx: mpsc::UnboundedSender<RunnerMsg>) -> ProcessHandle {
    let _runtime = crate::sys::runtime().enter();
    let start = Instant::now();
    let mut cmd = build_shell_command(&job);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(RunnerMsg::Line(format!("Failed to spawn: {}", e)));
            let _ = tx.send(RunnerMsg::Done(Some(1), start.elapsed()));
            let dummy = tokio::spawn(async {}).abort_handle();
            return ProcessHandle {
                supervisor: dummy.clone(),
                out_abort: dummy.clone(),
                err_abort: dummy,
                #[cfg(unix)]
                pid: None,
            };
        }
    };

    #[cfg(unix)]
    let pid = child.id();
    let (out_task, out_abort) = spawn_stream(child.stdout.take(), tx.clone());
    let (err_task, err_abort) = spawn_stream(child.stderr.take(), tx.clone());

    let supervisor = tokio::spawn(async move {
        let status = child.wait().await;
        let _ = out_task.await;
        let _ = err_task.await;
        let code = status.ok().and_then(|s| s.code());
        let _ = tx.send(RunnerMsg::Done(code, start.elapsed()));
    })
    .abort_handle();

    ProcessHandle {
        supervisor,
        out_abort,
        err_abort,
        #[cfg(unix)]
        pid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_command() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _handle = spawn_command(ShellJob::user("echo test_ok".into()), tx);

        let mut lines = Vec::new();
        let mut exit = None;

        while let Some(msg) = rx.recv().await {
            match msg {
                RunnerMsg::Line(l) => lines.push(l),
                RunnerMsg::Done(code, _) => {
                    exit = code;
                    break;
                }
            }
        }

        assert_eq!(lines, vec!["test_ok"]);
        assert_eq!(exit, Some(0));
    }

    #[tokio::test]
    async fn test_kill_command_in_tracks() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut handle = spawn_command(ShellJob::user("sleep 30".into()), tx);

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.kill();
    }
}
