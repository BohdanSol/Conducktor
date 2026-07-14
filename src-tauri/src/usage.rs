//! Talks to Claude Code's internal usage endpoint.
//!
//! This endpoint is not part of the public Claude API — it's what
//! Claude Code's own `/usage` screen calls. It may change without
//! notice, so parsing here is deliberately tolerant, and all the
//! reverse-engineered wire-shape quirks are isolated to this module:
//!
//! - `resets_at` on every bucket is an RFC 3339 timestamp string (e.g.
//!   `"2026-07-14T17:49:59.775019+00:00"`), not a Unix integer.
//! - There is no top-level `seven_day_fable` field. The Fable/Mythos
//!   weekly figure instead lives inside a `limits[]` array as a
//!   `weekly_scoped` entry whose `scope.model.display_name` names the
//!   model. `five_hour`/`seven_day` also have `limits[]` counterparts
//!   (kinds `session` / `weekly_all`), but the top-level fields already
//!   cover those, so `limits[]` is only consulted for the scoped entry.
//! - The response carries a long tail of other fields (`extra_usage`,
//!   `spend`, per-model buckets, and several fields with what look like
//!   internal codenames) that this app has no use for and ignores.
//!
//! The rest of the app only ever sees the normalized [`UsageResponse`] /
//! [`Bucket`] types below (`resets_at` as `Option<i64>` Unix seconds),
//! so none of this wire-format churn leaks past this file.

use serde::Deserialize;
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// Sent as-is on every request. Community reports indicate the endpoint
/// 429s aggressively without a recognized `User-Agent`. Kept as a single
/// constant so it's trivial to swap if this app's own UA gets rate
/// limited — see README for the fallback value that's been reported to
/// work reliably (a Claude-Code-style UA string).
const USER_AGENT: &str = "Conducktor-Claude-Usage-Tray/0.1.0";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Normalized bucket used by the rest of the app.
#[derive(Debug, Clone)]
pub struct Bucket {
    pub utilization: f64,
    /// Unix timestamp in seconds, already parsed from whatever wire
    /// format the endpoint used.
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct UsageResponse {
    pub five_hour: Option<Bucket>,
    pub seven_day: Option<Bucket>,
    /// Synthesized from the `weekly_scoped` entry in `limits[]` — see
    /// module docs. `None` if the account has no Fable/Mythos access.
    pub seven_day_fable: Option<Bucket>,
}

#[derive(Debug, Clone)]
pub enum UsageError {
    /// Token was rejected (401). Caller should re-read credentials and
    /// retry once before giving up.
    Unauthorized,
    /// Endpoint returned 429 — back off before trying again.
    RateLimited,
    /// Anything else: connection failure, unexpected status, bad JSON.
    Network(String),
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsageError::Unauthorized => write!(f, "unauthorized"),
            UsageError::RateLimited => write!(f, "rate limited"),
            UsageError::Network(msg) => write!(f, "network error: {msg}"),
        }
    }
}

// --- Raw wire shape (private; never leaves this module) ---

#[derive(Debug, Clone, Deserialize)]
struct RawBucket {
    utilization: f64,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawLimitModel {
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawLimitScope {
    #[serde(default)]
    model: Option<RawLimitModel>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawLimitEntry {
    kind: String,
    percent: f64,
    #[serde(default)]
    resets_at: Option<String>,
    #[serde(default)]
    scope: Option<RawLimitScope>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawUsageResponse {
    five_hour: Option<RawBucket>,
    seven_day: Option<RawBucket>,
    #[serde(default)]
    limits: Vec<RawLimitEntry>,
}

fn parse_resets_at(raw: &Option<String>) -> Option<i64> {
    raw.as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
}

impl From<RawBucket> for Bucket {
    fn from(raw: RawBucket) -> Self {
        Bucket {
            utilization: raw.utilization,
            resets_at: parse_resets_at(&raw.resets_at),
        }
    }
}

fn find_weekly_fable(limits: &[RawLimitEntry]) -> Option<Bucket> {
    limits.iter().find_map(|entry| {
        if entry.kind != "weekly_scoped" {
            return None;
        }
        let name = entry.scope.as_ref()?.model.as_ref()?.display_name.as_deref()?;
        let name_lower = name.to_lowercase();
        if name_lower.contains("fable") || name_lower.contains("mythos") {
            Some(Bucket {
                utilization: entry.percent,
                resets_at: parse_resets_at(&entry.resets_at),
            })
        } else {
            None
        }
    })
}

impl From<RawUsageResponse> for UsageResponse {
    fn from(raw: RawUsageResponse) -> Self {
        UsageResponse {
            seven_day_fable: find_weekly_fable(&raw.limits),
            five_hour: raw.five_hour.map(Bucket::from),
            seven_day: raw.seven_day.map(Bucket::from),
        }
    }
}

pub async fn fetch_usage(client: &reqwest::Client, token: &str) -> Result<UsageResponse, UsageError> {
    let resp = client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| UsageError::Network(e.to_string()))?;

    match resp.status() {
        reqwest::StatusCode::OK => {
            let text = resp
                .text()
                .await
                .map_err(|e| UsageError::Network(format!("bad response body: {e}")))?;
            let raw: RawUsageResponse = serde_json::from_str(&text)
                .map_err(|e| UsageError::Network(format!("bad response body: {e}")))?;
            Ok(UsageResponse::from(raw))
        }
        reqwest::StatusCode::UNAUTHORIZED => Err(UsageError::Unauthorized),
        reqwest::StatusCode::TOO_MANY_REQUESTS => Err(UsageError::RateLimited),
        status => Err(UsageError::Network(format!("unexpected status {status}"))),
    }
}
