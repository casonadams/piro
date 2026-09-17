use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub struct HttpRequest<'a> {
    pub url: &'a str,
    pub body: &'a str,
    pub auth_token: Option<&'a str>,
}

pub async fn post_json(req: HttpRequest<'_>) -> Result<String, String> {
    let mut cmd = Command::new("curl");
    cmd.arg("-s")
        .arg("-S")
        .arg("-X")
        .arg("POST")
        .arg(req.url)
        .arg("-H")
        .arg("Content-Type: application/json");

    if let Some(token) = req.auth_token {
        cmd.arg("-H").arg(format!("Authorization: Bearer {}", token));
    }

    cmd.arg("-d")
        .arg("@-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn curl: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(req.body.as_bytes())
            .await
            .map_err(|e| format!("Failed to write to curl: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("Failed to read curl output: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl error: {}", err));
    }

    String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 from curl: {}", e))
}
