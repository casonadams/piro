//! Paste-code login: the authorize URL and parsing of the code the user copies back.

use super::endpoints::{AUTHORIZE_URL, CLIENT_ID, REDIRECT_URI, SCOPES};
use crate::auth::pkce::{Pkce, random_token};

pub struct LoginRequest {
    pub url: String,
    pub(super) verifier: String,
    pub(super) state: String,
}

impl LoginRequest {
    pub fn begin() -> Self {
        let pkce = Pkce::generate();
        let state = random_token();
        let url = authorize_url(&pkce.challenge, &state);
        Self {
            url,
            verifier: pkce.verifier,
            state,
        }
    }
}

pub fn authorize_url(challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("valid authorize URL");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("code", "true");
    url.to_string()
}

/// Accepts `CODE#STATE`, a callback URL carrying `code`/`state` in its query, or a bare code.
pub fn parse_pasted_code(input: &str, expected_state: &str) -> Result<String, String> {
    let (code, state) = split_pasted(input.trim());
    if code.is_empty() {
        return Err("Authorization code cannot be empty".into());
    }
    if state.is_some_and(|s| s != expected_state) {
        return Err("OAuth state mismatch".into());
    }
    Ok(code)
}

fn split_pasted(input: &str) -> (String, Option<String>) {
    if let Some((_, query)) = input.split_once('?') {
        let (mut code, mut state) = (None, None);
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "state" => state = Some(value.into_owned()),
                _ => {}
            }
        }
        if let Some(code) = code {
            return (code, state);
        }
    }
    match input.split_once('#') {
        Some((code, state)) => (code.trim().to_string(), Some(state.trim().to_string())),
        None => (input.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn authorize_url_carries_pkce_state_and_code_flag() {
        let url = url::Url::parse(&authorize_url("CHAL", "ST")).unwrap();
        assert_eq!(url.host_str(), Some("claude.com"));
        assert_eq!(url.path(), "/cai/oauth/authorize");
        let params: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["client_id"], CLIENT_ID);
        assert_eq!(params["redirect_uri"], REDIRECT_URI);
        assert_eq!(params["scope"], SCOPES);
        assert_eq!(params["code_challenge"], "CHAL");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["state"], "ST");
        assert_eq!(params["code"], "true");
    }

    #[test]
    fn parses_pasted_codes() {
        let cases: [(&str, &str, Result<&str, &str>); 7] = [
            ("abc", "ST", Ok("abc")),
            ("  abc#ST  ", "ST", Ok("abc")),
            ("abc#OTHER", "ST", Err("OAuth state mismatch")),
            (
                "https://platform.claude.com/oauth/code/callback?code=abc%2B1&state=ST",
                "ST",
                Ok("abc+1"),
            ),
            ("?code=abc&state=OTHER", "ST", Err("OAuth state mismatch")),
            ("   ", "ST", Err("Authorization code cannot be empty")),
            ("#ST", "ST", Err("Authorization code cannot be empty")),
        ];
        for (input, state, want) in cases {
            let got = parse_pasted_code(input, state);
            assert_eq!(got, want.map(String::from).map_err(String::from), "input {input:?}");
        }
    }
}
