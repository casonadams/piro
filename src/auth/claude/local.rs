//! Imports the login Claude Code already holds, from `~/.claude` or the macOS keychain.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::auth::store::{StoredCredential, now_ms};

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const REFRESH_WINDOW_MS: i64 = 60_000;

/// Parses `.credentials.json` or the keychain payload. Expiry is normalised to epoch ms.
pub fn parse_credentials(raw: &str, account_email: Option<String>) -> Option<StoredCredential> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let target = value.get("claudeAiOauth").unwrap_or(&value);
    let non_empty = |v: Option<&Value>| v?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let access = non_empty(target.get("accessToken").or_else(|| target.get("access_token")))?;
    let refresh = non_empty(target.get("refreshToken").or_else(|| target.get("refresh_token")));
    Some(StoredCredential::OAuth {
        access,
        refresh,
        expires: parse_expires(target.get("expiresAt").or_else(|| target.get("expires_at"))),
        account_email,
    })
}

fn parse_expires(value: Option<&Value>) -> Option<i64> {
    let num = value?.as_i64()?;
    Some(if num > 10_000_000_000 { num } else { num * 1000 })
}

pub fn parse_account_email(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    value
        .pointer("/oauthAccount/emailAddress")
        .and_then(Value::as_str)
        .map(String::from)
}

/// Claude Code's file credential when it is still fresh, else the keychain copy, else the file one.
pub async fn detect() -> Option<StoredCredential> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let email = read_email(&home.join(".claude.json")).await;
    let file = read_file(&home.join(".claude").join(".credentials.json"), email.clone()).await;
    if let Some(cred) = file.as_ref().filter(|c| !c.expires_within(now_ms(), REFRESH_WINDOW_MS)) {
        return Some(cred.clone());
    }
    let keychain = keychain_payload().await.and_then(|raw| parse_credentials(&raw, email));
    keychain.or(file)
}

async fn read_file(path: &Path, email: Option<String>) -> Option<StoredCredential> {
    let raw = tokio::fs::read_to_string(path).await.ok()?;
    parse_credentials(&raw, email)
}

async fn read_email(path: &Path) -> Option<String> {
    let raw = tokio::fs::read_to_string(path).await.ok()?;
    parse_account_email(&raw)
}

#[cfg(target_os = "macos")]
async fn keychain_payload() -> Option<String> {
    let output = tokio::process::Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    decode_keychain(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
async fn keychain_payload() -> Option<String> {
    None
}

/// `security` prints the JSON as-is or, for some items, as hex-encoded UTF-8.
#[cfg(any(target_os = "macos", test))]
fn decode_keychain(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return Some(trimmed.to_string());
    }
    let bytes = (0..trimmed.len())
        .step_by(2)
        .map(|i| trimmed.get(i..i + 2).and_then(|hex| u8::from_str_radix(hex, 16).ok()))
        .collect::<Option<Vec<u8>>>()?;
    let text = String::from_utf8(bytes).ok()?;
    let text = text.trim();
    text.starts_with('{').then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_credentials_with_millisecond_expiry() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"at","refreshToken":"rt","expiresAt":1700000000000}}"#;
        assert_eq!(
            parse_credentials(raw, Some("me@x".into())),
            Some(StoredCredential::OAuth {
                access: "at".into(),
                refresh: Some("rt".into()),
                expires: Some(1_700_000_000_000),
                account_email: Some("me@x".into()),
            })
        );
    }

    #[test]
    fn normalises_second_expiry_and_accepts_flat_keys() {
        let raw = r#"{"access_token":"at","expires_at":1700000000}"#;
        let Some(StoredCredential::OAuth { refresh, expires, .. }) = parse_credentials(raw, None) else {
            panic!("expected oauth");
        };
        assert_eq!(refresh, None);
        assert_eq!(expires, Some(1_700_000_000_000));
    }

    #[test]
    fn rejects_missing_or_blank_access_token() {
        assert_eq!(
            parse_credentials(r#"{"claudeAiOauth":{"accessToken":"  "}}"#, None),
            None
        );
        assert_eq!(parse_credentials("not json", None), None);
    }

    #[test]
    fn reads_account_email_from_config() {
        let raw = r#"{"oauthAccount":{"emailAddress":"a@b.c"}}"#;
        assert_eq!(parse_account_email(raw).as_deref(), Some("a@b.c"));
        assert_eq!(parse_account_email("{}"), None);
    }

    #[test]
    fn decodes_keychain_json_and_hex_payloads() {
        assert_eq!(decode_keychain("{\"a\":1}\n").as_deref(), Some("{\"a\":1}"));
        assert_eq!(decode_keychain("7b2261223a317d\n").as_deref(), Some("{\"a\":1}"));
        assert_eq!(decode_keychain("zz"), None);
        assert_eq!(decode_keychain("é"), None);
    }
}
