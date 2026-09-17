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

pub fn spawn_command(cmd_str: String, tx: mpsc::UnboundedSender<RunnerMsg>) -> AbortHandle {
    tokio::spawn(async move {
        let start = Instant::now();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut child = match Command::new(shell)
            .arg("-c")
            .arg(&cmd_str)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(RunnerMsg::Line(format!("Failed to spawn: {}", e)));
                let _ = tx.send(RunnerMsg::Done(Some(1), start.elapsed()));
                return;
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let tx_out = tx.clone();
        let tx_err = tx.clone();

        let out_task = tokio::spawn(async move {
            if let Some(out) = stdout {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let clean = strip_ansi_escapes::strip(line.as_bytes());
                    if tx_out
                        .send(RunnerMsg::Line(String::from_utf8_lossy(&clean).into_owned()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });

        let err_task = tokio::spawn(async move {
            if let Some(err) = stderr {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let clean = strip_ansi_escapes::strip(line.as_bytes());
                    if tx_err
                        .send(RunnerMsg::Line(String::from_utf8_lossy(&clean).into_owned()))
                        .is_err()
                    {
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
    .abort_handle()
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
}
