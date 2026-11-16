//! Claude subscription OAuth: paste-code login, import of Claude Code's login, and token refresh.

mod endpoints;
mod flow;
mod local;
mod tokens;

use std::path::Path;

pub use endpoints::{ANTHROPIC_BETA, USER_AGENT};
pub use flow::LoginRequest;

use crate::auth::store::{AuthStore, StoredCredential, default_path, now_ms};
use endpoints::{PROFILE_URL, TOKEN_URL};
use flow::parse_pasted_code;
use tokens::{CodeGrant, exchange_code, fetch_email, refresh_credential};

pub const PROVIDER: &str = "claude";
const REFRESH_WINDOW_MS: i64 = 60_000;
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn begin_login() -> LoginRequest {
    LoginRequest::begin()
}

/// Exchanges the pasted code, stores the credential, and returns the account email when known.
pub async fn complete_login(request: &LoginRequest, pasted: &str) -> Result<Option<String>, String> {
    let code = parse_pasted_code(pasted, &request.state)?;
    let grant = CodeGrant {
        code: &code,
        verifier: &request.verifier,
        state: &request.state,
    };
    let token = exchange_code(TOKEN_URL, &grant).await?;
    let email = fetch_email(PROFILE_URL, &token.access_token).await;
    let cred = token.into_credential(None, email);
    let email = cred.account_email().map(String::from);
    save_credential(&default_path()?, cred)?;
    Ok(email)
}

pub async fn has_local_credentials() -> bool {
    local::detect().await.is_some()
}

/// Copies Claude Code's login into the piro store; `Ok(false)` when there is none to import.
pub async fn import_local_credentials() -> Result<bool, String> {
    let Some(cred) = local::detect().await else {
        return Ok(false);
    };
    save_credential(&default_path()?, cred)?;
    Ok(true)
}

pub fn open_browser(url: &str) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not run {program}: {e}"))
}

/// A bearer token for requests, refreshing first when it is near expiry or when `stale` is the token
/// the server just rejected. The store is re-read under a lock, so a token another process rotated is reused.
pub async fn access_token(store_path: &Path, stale: Option<&str>) -> Result<String, String> {
    let _guard = REFRESH_LOCK.lock().await;
    let mut store = AuthStore::open(store_path)?;
    let cred = store
        .get(PROVIDER)
        .cloned()
        .ok_or_else(|| "No Claude credentials found. Run 'piro login claude'.".to_string())?;
    if !needs_refresh(&cred, stale, now_ms()) {
        return Ok(cred.secret().to_string());
    }
    let refreshed = refresh_credential(&cred)
        .await
        .map_err(|e| format!("Claude session could not be refreshed: {e}. Run 'piro login claude'."))?;
    let token = refreshed.secret().to_string();
    store.set(PROVIDER, refreshed);
    store.save()?;
    Ok(token)
}

fn needs_refresh(cred: &StoredCredential, stale: Option<&str>, now_ms: i64) -> bool {
    matches!(cred, StoredCredential::OAuth { .. })
        && (cred.expires_within(now_ms, REFRESH_WINDOW_MS) || stale == Some(cred.secret()))
}

fn save_credential(path: &Path, cred: StoredCredential) -> Result<(), String> {
    let mut store = AuthStore::open(path)?;
    store.set(PROVIDER, cred);
    store.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth(expires: Option<i64>) -> StoredCredential {
        StoredCredential::OAuth {
            access: "tok".into(),
            refresh: Some("r".into()),
            expires,
            account_email: None,
        }
    }

    #[test]
    fn refreshes_when_expiring_or_when_the_server_rejected_the_token() {
        let far = Some(10_000_000);
        assert!(!needs_refresh(&oauth(far), None, 0));
        assert!(needs_refresh(&oauth(Some(30_000)), None, 0));
        assert!(needs_refresh(&oauth(far), Some("tok"), 0));
        assert!(!needs_refresh(&oauth(far), Some("older-token"), 0));
        assert!(!needs_refresh(
            &StoredCredential::ApiKey { key: "k".into() },
            Some("k"),
            0
        ));
    }
}
