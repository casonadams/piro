use std::sync::OnceLock;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Response};

pub struct HttpRequest<'a> {
    pub url: &'a str,
    pub body: &'a str,
    pub auth_token: Option<&'a str>,
}

fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| Client::builder().build().unwrap_or_default())
}

pub async fn post_request(req: HttpRequest<'_>) -> Result<Response, String> {
    let mut builder = client()
        .post(req.url)
        .header(CONTENT_TYPE, "application/json")
        .body(req.body.to_string());

    if let Some(token) = req.auth_token {
        builder = builder.bearer_auth(token);
    }

    builder.send().await.map_err(|e| format!("HTTP POST error: {}", e))
}

pub async fn post_json(req: HttpRequest<'_>) -> Result<String, String> {
    let res = post_request(req).await?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("HTTP error {}: {}", status, body));
    }
    res.text().await.map_err(|e| format!("Failed to read response: {}", e))
}
