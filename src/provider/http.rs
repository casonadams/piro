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
