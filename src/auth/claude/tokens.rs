//! Token endpoint, profile lookup, and the stored credential they produce.

use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::endpoints::{ANTHROPIC_BETA, CLIENT_ID, OAUTH_TIMEOUT, REDIRECT_URI, TOKEN_URL, USER_AGENT};
use crate::auth::store::{StoredCredential, now_ms};
use crate::provider::http::client;

const SKEW_MS: i64 = 5 * 60 * 1000;

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
    pub account: Option<Account>,
}

#[derive(Deserialize)]
pub struct Account {
    pub email_address: Option<String>,
}

#[derive(Deserialize)]
struct Profile {
    account: Option<Account>,
}

pub struct CodeGrant<'a> {
    pub code: &'a str,
    pub verifier: &'a str,
    pub state: &'a str,
}

impl TokenResponse {
    /// A response without a refresh token keeps the previous one; the email falls back the same way.
    pub fn into_credential(
        self,
        previous: Option<&StoredCredential>,
        profile_email: Option<String>,
    ) -> StoredCredential {
        let (previous_refresh, previous_email) = match previous {
            Some(StoredCredential::OAuth {
                refresh, account_email, ..
            }) => (refresh.clone(), account_email.clone()),
            _ => (None, None),
        };
        StoredCredential::OAuth {
            access: self.access_token,
            refresh: self.refresh_token.or(previous_refresh),
            expires: self.expires_in.map(|secs| expires_at(now_ms(), secs)),
            account_email: profile_email
                .or(self.account.and_then(|a| a.email_address))
                .or(previous_email),
        }
    }
}

/// Access-token expiry in epoch ms, pulled 5 minutes early so refreshes land before the server rejects it.
fn expires_at(now: i64, expires_in_secs: i64) -> i64 {
    now + expires_in_secs * 1000 - SKEW_MS
}

pub async fn exchange_code(token_url: &str, grant: &CodeGrant<'_>) -> Result<TokenResponse, String> {
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", grant.code),
        ("code_verifier", grant.verifier),
        ("redirect_uri", REDIRECT_URI),
        ("state", grant.state),
    ];
    post_form(token_url, &form, "Claude token exchange").await
}

pub async fn refresh_credential(previous: &StoredCredential) -> Result<StoredCredential, String> {
    let StoredCredential::OAuth {
        refresh: Some(refresh), ..
    } = previous
    else {
        return Err("Claude OAuth token has no refresh token".into());
    };
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", CLIENT_ID),
        ("refresh_token", refresh.as_str()),
    ];
    let token: TokenResponse = post_form(TOKEN_URL, &form, "Claude token refresh").await?;
    Ok(token.into_credential(Some(previous), None))
}

/// Best-effort: a failed profile lookup just leaves the email unknown.
pub async fn fetch_email(profile_url: &str, access_token: &str) -> Option<String> {
    let res = client()
        .get(profile_url)
        .timeout(OAUTH_TIMEOUT)
        .bearer_auth(access_token)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header("anthropic-beta", ANTHROPIC_BETA)
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let profile: Profile = res.json().await.ok()?;
    profile.account?.email_address
}

async fn post_form<T: DeserializeOwned>(url: &str, form: &[(&str, &str)], operation: &str) -> Result<T, String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let res = client()
        .post(url)
        .timeout(OAUTH_TIMEOUT)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("{operation} network error: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!("{operation} failed ({status}): {body}"));
    }
    res.json()
        .await
        .map_err(|e| format!("Failed to parse {operation} response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(json: &str) -> TokenResponse {
        serde_json::from_str(json).unwrap()
    }

    fn oauth(refresh: Option<&str>, email: Option<&str>) -> StoredCredential {
        StoredCredential::OAuth {
            access: "old".into(),
            refresh: refresh.map(String::from),
            expires: None,
            account_email: email.map(String::from),
        }
    }

    #[test]
    fn expiry_is_net_of_the_skew() {
        assert_eq!(expires_at(1_000_000, 3600), 1_000_000 + 3_600_000 - 300_000);
    }

    #[test]
    fn parses_a_response_without_refresh_token() {
        let cred = token(r#"{"access_token":"new","expires_in":3600}"#).into_credential(None, None);
        let StoredCredential::OAuth { access, refresh, .. } = cred else {
            panic!("expected oauth");
        };
        assert_eq!(access, "new");
        assert_eq!(refresh, None);
    }

    #[test]
    fn refresh_without_rotation_keeps_previous_refresh_token_and_email() {
        let prev = oauth(Some("r1"), Some("a@b.c"));
        let cred = token(r#"{"access_token":"new"}"#).into_credential(Some(&prev), None);
        let StoredCredential::OAuth {
            refresh, account_email, ..
        } = cred
        else {
            panic!("expected oauth");
        };
        assert_eq!(refresh.as_deref(), Some("r1"));
        assert_eq!(account_email.as_deref(), Some("a@b.c"));
    }

    #[test]
    fn rotated_refresh_token_and_profile_email_win() {
        let prev = oauth(Some("r1"), Some("old@b.c"));
        let cred = token(r#"{"access_token":"new","refresh_token":"r2","account":{"email_address":"tok@b.c"}}"#)
            .into_credential(Some(&prev), Some("profile@b.c".into()));
        let StoredCredential::OAuth {
            refresh, account_email, ..
        } = cred
        else {
            panic!("expected oauth");
        };
        assert_eq!(refresh.as_deref(), Some("r2"));
        assert_eq!(account_email.as_deref(), Some("profile@b.c"));
    }
}
