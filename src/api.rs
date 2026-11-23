//! The usage endpoint and the token refresh: their JSON and the HTTP calls.

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use jiff::Timestamp;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// One rate-limit window: how much of it is used and when it starts over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    /// Percent used, 0 to 100.
    #[serde(default, deserialize_with = "null_as_zero")]
    pub utilization: f64,
    pub resets_at: Option<Timestamp>,
}

/// What `/api/oauth/usage` reports. A missing window means no such limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub five_hour: Option<Window>,
    #[serde(default)]
    pub seven_day: Option<Window>,
}

fn null_as_zero<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(0.0))
}

/// A refresh response. The refresh token is only present when it rotated.
#[derive(Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds the access token lives; Claude's tokens last 8 hours.
    #[serde(default = "eight_hours")]
    pub expires_in: i64,
}

fn eight_hours() -> i64 {
    8 * 60 * 60
}

// Never print the tokens, not even through `{:?}`.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

#[derive(Debug)]
pub enum ApiError {
    /// The token was refused: log in again (or refresh).
    Unauthorized,
    Other(anyhow::Error),
}

pub fn parse_usage(body: &str) -> Result<Usage> {
    serde_json::from_str(body).context("i couldn't read the usage response")
}

pub fn parse_tokens(body: &str) -> Result<Tokens> {
    serde_json::from_str(body).context("i couldn't read the token response")
}

/// Maps a response to an error, or `None` when it is a success. Only the
/// status and the `error` field are looked at, so no secret can leak.
fn classify(status: u16, body: &str, refresh: bool) -> Option<ApiError> {
    if (200..300).contains(&status) {
        return None;
    }
    let rejected_grant = refresh
        && status == 400
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .is_some_and(|v| v["error"] == "invalid_grant");
    if status == 401 || rejected_grant {
        return Some(ApiError::Unauthorized);
    }
    Some(ApiError::Other(anyhow::anyhow!(
        "the server answered {status}"
    )))
}

fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("the http client builds")
    })
}

/// Sends a request and returns the body of a successful answer.
fn send(req: reqwest::blocking::RequestBuilder, refresh: bool) -> Result<String, ApiError> {
    let res = req.send().map_err(|e| {
        ApiError::Other(anyhow::anyhow!(
            "i couldn't reach the server: {}",
            e.without_url()
        ))
    })?;
    let status = res.status().as_u16();
    let body = res.text().map_err(|e| {
        ApiError::Other(anyhow::anyhow!(
            "i couldn't read the answer: {}",
            e.without_url()
        ))
    })?;
    match classify(status, &body, refresh) {
        Some(err) => Err(err),
        None => Ok(body),
    }
}

pub fn fetch_usage(token: &str) -> Result<Usage, ApiError> {
    let req = client()
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20");
    parse_usage(&send(req, false)?).map_err(ApiError::Other)
}

pub fn refresh(refresh_token: &str) -> Result<Tokens, ApiError> {
    let req = client().post(TOKEN_URL).json(&serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    }));
    parse_tokens(&send(req, true)?).map_err(ApiError::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USAGE: &str = include_str!("../tests/fixtures/usage.json");
    const NO_FIVE_HOUR: &str = include_str!("../tests/fixtures/usage_no_five_hour.json");

    #[test]
    fn parses_recorded_usage() {
        let usage = parse_usage(USAGE).unwrap();
        let five = usage.five_hour.unwrap();
        assert_eq!(five.utilization, 6.0);
        assert_eq!(
            five.resets_at,
            Some("2026-11-23T17:30:00.243657Z".parse().unwrap())
        );
        let seven = usage.seven_day.unwrap();
        assert_eq!(seven.utilization, 94.0);
        assert_eq!(
            seven.resets_at,
            Some("2026-11-25T14:00:00.243676Z".parse().unwrap())
        );
    }

    #[test]
    fn null_five_hour_is_none() {
        let usage = parse_usage(NO_FIVE_HOUR).unwrap();
        assert!(usage.five_hour.is_none());
        assert_eq!(usage.seven_day.unwrap().utilization, 94.0);
    }

    #[test]
    fn null_or_missing_utilization_is_zero() {
        let usage =
            parse_usage(r#"{"five_hour":{"utilization":null,"resets_at":null},"seven_day":{}}"#)
                .unwrap();
        assert_eq!(usage.five_hour.unwrap().utilization, 0.0);
        assert_eq!(usage.seven_day.unwrap().utilization, 0.0);
    }

    #[test]
    fn parses_tokens() {
        let t =
            parse_tokens(r#"{"access_token":"a","refresh_token":"r","expires_in":28800}"#).unwrap();
        assert_eq!(t.access_token, "a");
        assert_eq!(t.refresh_token.as_deref(), Some("r"));
        assert_eq!(t.expires_in, 28800);
    }

    #[test]
    fn missing_expires_in_defaults_to_eight_hours() {
        let t = parse_tokens(r#"{"access_token":"a","refresh_token":"r"}"#).unwrap();
        assert_eq!(t.expires_in, 28800);
    }

    #[test]
    fn tokens_debug_hides_secrets() {
        let t = parse_tokens(
            r#"{"access_token":"sekret-access","refresh_token":"sekret-refresh","expires_in":1}"#,
        )
        .unwrap();
        let shown = format!("{t:?}");
        assert!(!shown.contains("sekret"));
        assert!(shown.contains("<redacted>"));
    }

    #[test]
    fn tokens_without_refresh_token() {
        let t = parse_tokens(r#"{"access_token":"a","expires_in":10}"#).unwrap();
        assert_eq!(t.refresh_token, None);
    }

    #[test]
    fn classifies_statuses() {
        assert!(classify(200, "", false).is_none());
        assert!(matches!(
            classify(401, "", false),
            Some(ApiError::Unauthorized)
        ));
        assert!(matches!(
            classify(401, "", true),
            Some(ApiError::Unauthorized)
        ));
        assert!(matches!(
            classify(400, r#"{"error":"invalid_grant"}"#, true),
            Some(ApiError::Unauthorized)
        ));
        assert!(matches!(
            classify(400, r#"{"error":"invalid_request"}"#, true),
            Some(ApiError::Other(_))
        ));
        assert!(matches!(
            classify(400, r#"{"error":"invalid_grant"}"#, false),
            Some(ApiError::Other(_))
        ));
        assert!(matches!(classify(500, "", false), Some(ApiError::Other(_))));
    }
}
