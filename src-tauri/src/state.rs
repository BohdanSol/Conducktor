use std::sync::Mutex;
use std::time::SystemTime;
use tokio::sync::Notify;

use crate::credentials::Credentials;
use crate::settings::Settings;
use crate::usage::UsageResponse;

/// Snapshot of the most recent fetch attempt, good or bad.
///
/// `response` always holds the last *successful* payload, even while a
/// subsequent fetch is failing — the tray keeps showing last-known-good
/// data with a staleness marker rather than blanking out.
pub struct LatestUsage {
    pub response: Option<UsageResponse>,
    pub fetched_at: Option<SystemTime>,
    /// Machine-readable status of the most recent attempt.
    pub status: FetchStatus,
}

impl Default for LatestUsage {
    fn default() -> Self {
        Self {
            response: None,
            fetched_at: None,
            status: FetchStatus::NoDataYet,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchStatus {
    /// Most recent fetch succeeded.
    Ok,
    /// We have prior good data, but the last attempt failed (network/rate limit).
    Stale,
    /// No Claude Code login could be found at all.
    NotLoggedIn,
    /// Login was found but the token was rejected even after a refresh retry.
    Unauthorized,
    /// App just started and hasn't completed a fetch yet.
    NoDataYet,
}

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub credentials: Mutex<Option<Credentials>>,
    pub latest: Mutex<LatestUsage>,
    /// Wakes the poll loop early (Settings saved / Refresh Now clicked).
    pub notify: Notify,
    /// Set by `refresh_now`; consumed by the poll loop to force an
    /// immediate network fetch on the next tick regardless of the interval.
    pub force_refresh: Mutex<bool>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            settings: Mutex::new(Settings::default()),
            credentials: Mutex::new(None),
            latest: Mutex::new(LatestUsage::default()),
            notify: Notify::new(),
            force_refresh: Mutex::new(false),
        }
    }
}
