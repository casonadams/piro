//! Messages API headers and the friendly text for failed responses.

use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::Value;

use super::AuthMode;
use crate::auth::claude::{ANTHROPIC_BETA, USER_AGENT as CLI_USER_AGENT};

pub const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// `status` is `None` when no response arrived at all.
pub struct ApiError {
    pub status: Option<u16>,
    pub body: String,
    pub retry_after: Option<Duration>,
}

impl ApiError {
    pub fn transport(message: impl Into<String>) -> Self {
        Self {
            status: None,
            body: message.into(),
            retry_after: None,
        }
    }
}

pub fn request_headers(mode: AuthMode, token: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("anthropic-version"),
        HeaderValue::from_static(ANTHROPIC_VERSION),
    );
    match mode {
        AuthMode::ApiKey => {
            headers.insert(HeaderName::from_static("x-api-key"), header_value(token)?);
        }
        AuthMode::OAuth => {
            headers.insert(AUTHORIZATION, header_value(&format!("Bearer {token}"))?);
            headers.insert(
                HeaderName::from_static("anthropic-beta"),
                HeaderValue::from_static(ANTHROPIC_BETA),
            );
            headers.insert(
                HeaderName::from_static("anthropic-dangerous-direct-browser-access"),
                HeaderValue::from_static("true"),
            );
            headers.insert(HeaderName::from_static("x-app"), HeaderValue::from_static("cli"));
            headers.insert(USER_AGENT, HeaderValue::from_static(CLI_USER_AGENT));
        }
    }
    Ok(headers)
}

fn header_value(value: &str) -> Result<HeaderValue, String> {
    HeaderValue::from_str(value).map_err(|_| "Credential contains characters not allowed in an HTTP header".to_string())
}

pub fn friendly_error(err: &ApiError, mode: AuthMode) -> String {
    let Some(status) = err.status else {
        return format!("Claude request failed: {}", err.body);
    };
    let detail = backend_message(&err.body);
    match status {
        401 if mode == AuthMode::OAuth => {
            "Claude OAuth session expired or credentials are invalid. Run 'piro login claude'.".to_string()
        }
        401 => "Anthropic API key was rejected. Check ANTHROPIC_API_KEY.".to_string(),
        403 => format!("Claude access denied: {detail}"),
        429 => format!("Claude rate or usage limit reached; wait a bit and retry: {detail}"),
        502 | 503 => "Claude service is temporarily unavailable. Wait a bit and retry.".to_string(),
        529 => "Claude API is overloaded. Wait a bit and retry.".to_string(),
        _ => format!("Claude API error ({status}): {detail}"),
    }
}

fn backend_message(body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message")?.as_str().map(String::from));
    parsed.unwrap_or_else(|| {
        let snippet: String = body.chars().take(300).collect();
        if snippet.trim().is_empty() {
            "unknown error".to_string()
        } else {
            snippet
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_headers_carry_bearer_and_beta_identity() {
        let headers = request_headers(AuthMode::OAuth, "tok").unwrap();
        assert_eq!(headers[AUTHORIZATION], "Bearer tok");
        assert_eq!(headers["anthropic-beta"], ANTHROPIC_BETA);
        assert_eq!(headers["x-app"], "cli");
        assert_eq!(headers[USER_AGENT], CLI_USER_AGENT);
        assert!(headers.get("x-api-key").is_none());
    }

    #[test]
    fn api_key_headers_use_x_api_key_only() {
        let headers = request_headers(AuthMode::ApiKey, "sk-1").unwrap();
        assert_eq!(headers["x-api-key"], "sk-1");
        assert_eq!(headers["anthropic-version"], ANTHROPIC_VERSION);
        assert!(headers.get(AUTHORIZATION).is_none());
        assert!(headers.get("anthropic-beta").is_none());
        assert!(request_headers(AuthMode::ApiKey, "bad\ntoken").is_err());
    }

    #[test]
    fn maps_statuses_to_friendly_text() {
        let err = |status| ApiError {
            status: Some(status),
            body: r#"{"error":{"message":"nope"}}"#.into(),
            retry_after: None,
        };
        assert!(friendly_error(&err(401), AuthMode::OAuth).contains("piro login claude"));
        assert!(friendly_error(&err(401), AuthMode::ApiKey).contains("ANTHROPIC_API_KEY"));
        assert!(friendly_error(&err(429), AuthMode::OAuth).contains("rate or usage limit"));
        assert!(friendly_error(&err(529), AuthMode::OAuth).contains("overloaded"));
        assert!(friendly_error(&err(400), AuthMode::OAuth).ends_with("nope"));
    }
}
