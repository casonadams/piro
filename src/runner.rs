use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

#[derive(Debug)]
pub enum RunnerMsg {
    Line(String),
    Done(Option<i32>, Duration),
}

pub struct ProcessHandle {
    abort: AbortHandle,
    #[cfg(unix)]
    pid: Option<u32>,
}

impl ProcessHandle {
    #[cfg(test)]
    pub fn mock(abort: AbortHandle) -> Self {
        Self {
            abort,
            #[cfg(unix)]
            pid: None,
        }
    }

    pub fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        self.abort.abort();
    }
}

pub fn spawn_command(cmd_str: String, tx: mpsc::UnboundedSender<RunnerMsg>) -> ProcessHandle {
    let start = Instant::now();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = Command::new(shell);
    cmd.arg("-c")
        .arg(&cmd_str)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("CLICOLOR", "1")
        .env("CLICOLOR_FORCE", "1")
        .env("FORCE_COLOR", "1")
        .env("COLORTERM", "truecolor")
        .env("TERM", "xterm-256color");

    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(RunnerMsg::Line(format!("Failed to spawn: {}", e)));
            let _ = tx.send(RunnerMsg::Done(Some(1), start.elapsed()));
            return ProcessHandle {
                abort: tokio::spawn(async {}).abort_handle(),
                #[cfg(unix)]
                pid: None,
            };
        }
    };

    #[cfg(unix)]
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let abort = tokio::spawn(async move {
        let tx_out = tx.clone();
        let tx_err = tx.clone();

        let out_task = tokio::spawn(async move {
            if let Some(out) = stdout {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if tx_out.send(RunnerMsg::Line(line)).is_err() {
                        break;
                    }
                }
            }
        });

        let err_task = tokio::spawn(async move {
            if let Some(err) = stderr {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if tx_err.send(RunnerMsg::Line(line)).is_err() {
                        break;
                    }
                }
            }
        });

        let status = child.wait().await;
        let _ = out_task.await;
        let _ = err_task.await;
        let code = status.ok().and_then(|s| s.code());
        let _ = tx.send(RunnerMsg::Done(code, start.elapsed()));
    })
    .abort_handle();

    ProcessHandle {
        abort,
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
        let _handle = spawn_command("echo test_ok".into(), tx);

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
        let mut handle = spawn_command("sleep 30".into(), tx);

        // Sleep 50ms then kill immediately
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.kill();
    }
}
