use std::sync::LazyLock;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use reqwest::{Client, Response};

use super::ProviderError;

pub struct HttpRequest<'a> {
    pub url: &'a str,
    pub body: String,
    pub auth_token: Option<&'a str>,
}

static CLIENT: LazyLock<Client> = LazyLock::new(|| Client::builder().build().unwrap_or_default());

pub fn client() -> &'static Client {
    &CLIENT
}

pub async fn post_request(req: HttpRequest<'_>) -> Result<Response, reqwest::Error> {
    let mut builder = client()
        .post(req.url)
        .header(CONTENT_TYPE, "application/json")
        .body(req.body);

    if let Some(token) = req.auth_token {
        builder = builder.bearer_auth(token);
    }

    builder.send().await
}

/// Rate limits, timeouts, server errors and Anthropic's 529 "overloaded" may pass on a later attempt.
pub fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500..=504 | 529)
}

pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let secs: f64 = headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

pub async fn status_error(res: Response, describe: impl FnOnce(u16, &str) -> String) -> ProviderError {
    let status = res.status().as_u16();
    let retry_after = retry_after(res.headers());
    let body = res.text().await.unwrap_or_default();
    ProviderError {
        message: describe(status, &body),
        retryable: retryable_status(status),
        retry_after,
    }
}
