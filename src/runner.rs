use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

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
        if let Some(s) = stream {
            let mut reader = BufReader::new(s).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(RunnerMsg::Line(line)).is_err() {
                    break;
                }
            }
        }
    });
    let abort = task.abort_handle();
    (task, abort)
}

pub fn spawn_command(cmd_str: String, tx: mpsc::UnboundedSender<RunnerMsg>) -> ProcessHandle {
    let start = Instant::now();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = Command::new(shell);
    cmd.arg("-c")
        .arg(&cmd_str)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("CLICOLOR", "1")
        .env("CLICOLOR_FORCE", "1")
        .env("FORCE_COLOR", "1")
        .env("COLORTERM", "truecolor")
        .env("TERM", "xterm-256color")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat");

    #[cfg(unix)]
    cmd.process_group(0);

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

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.kill();
    }
}
