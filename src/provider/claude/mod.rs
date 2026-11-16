//! Anthropic Messages API, authenticated with a Claude subscription login or an API key.

mod http;
mod request;
mod stream;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use reqwest::Response;
use serde_json::Value;

use crate::agent::AssistantTurn;
use crate::auth::claude::access_token;
use crate::provider::http::client;
use crate::provider::sse::for_each_line;
use crate::provider::{ChatRequestParams, LlmProvider};
use http::{ApiError, MESSAGES_URL, friendly_error, request_headers};
use stream::StreamAccumulator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    OAuth,
    ApiKey,
}

enum Credentials {
    OAuth { store_path: PathBuf },
    ApiKey(String),
}

pub struct ClaudeProvider {
    model: String,
    credentials: Credentials,
}

impl ClaudeProvider {
    pub fn oauth(model: &str, store_path: PathBuf) -> Self {
        Self::new(model, Credentials::OAuth { store_path })
    }

    pub fn api_key(model: &str, key: String) -> Self {
        Self::new(model, Credentials::ApiKey(key))
    }

    fn new(model: &str, credentials: Credentials) -> Self {
        Self {
            model: request::normalize_model_alias(model).to_string(),
            credentials,
        }
    }

    fn mode(&self) -> AuthMode {
        match self.credentials {
            Credentials::OAuth { .. } => AuthMode::OAuth,
            Credentials::ApiKey(_) => AuthMode::ApiKey,
        }
    }

    async fn credential(&self, stale: Option<&str>) -> Result<String, String> {
        match &self.credentials {
            Credentials::OAuth { store_path } => access_token(store_path, stale).await,
            Credentials::ApiKey(key) => Ok(key.clone()),
        }
    }

    async fn post(&self, token: &str, body: &Value) -> Result<Response, ApiError> {
        let headers = request_headers(self.mode(), token).map_err(ApiError::transport)?;
        let res = client()
            .post(MESSAGES_URL)
            .headers(headers)
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| ApiError::transport(e.to_string()))?;
        let status = res.status();
        if status.is_success() {
            return Ok(res);
        }
        let body = res.text().await.unwrap_or_default();
        Err(ApiError {
            status: Some(status.as_u16()),
            body,
        })
    }

    /// A 401 on a subscription login refreshes the token once and retries; the retry's failure is final.
    async fn send_with_refresh(&self, body: &Value) -> Result<Response, String> {
        let mut token = self.credential(None).await?;
        let mut result = self.post(&token, body).await;
        if self.mode() == AuthMode::OAuth && result.as_ref().is_err_and(|e| e.status == Some(401)) {
            token = self.credential(Some(&token)).await?;
            result = self.post(&token, body).await;
        }
        result.map_err(|e| friendly_error(&e, self.mode()))
    }
}

impl LlmProvider for ClaudeProvider {
    async fn chat_stream(&self, params: ChatRequestParams<'_>) -> Result<AssistantTurn, String> {
        let body = request::build_body(&self.model, self.mode(), &params);
        let res = self.send_with_refresh(&body).await?;
        let mut acc = StreamAccumulator::default();
        for_each_line(res, |line| acc.process_line(line, params.delta_tx)).await?;
        Ok(acc.finish())
    }
}
