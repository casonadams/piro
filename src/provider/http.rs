use std::sync::LazyLock;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Response};

pub struct HttpRequest<'a> {
    pub url: &'a str,
    pub body: String,
    pub auth_token: Option<&'a str>,
}

static CLIENT: LazyLock<Client> = LazyLock::new(|| Client::builder().build().unwrap_or_default());

pub fn client() -> &'static Client {
    &CLIENT
}

pub async fn post_request(req: HttpRequest<'_>) -> Result<Response, String> {
    let mut builder = client()
        .post(req.url)
        .header(CONTENT_TYPE, "application/json")
        .body(req.body);

    if let Some(token) = req.auth_token {
        builder = builder.bearer_auth(token);
    }

    builder.send().await.map_err(|e| format!("HTTP POST error: {}", e))
}
