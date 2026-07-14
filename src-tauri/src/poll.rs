use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::credentials::{self, CredError};
use crate::format;
use crate::settings::MIN_REFRESH_INTERVAL_SECS;
use crate::state::{AppState, FetchStatus};
use crate::usage::{self, UsageError};

/// Base cadence for the loop's own tick — independent of the configured
/// fetch interval. Keeps tray countdowns ("in 42m") fresh between
/// network fetches without hammering the endpoint.
const TICK: Duration = Duration::from_secs(30);

/// How many extra ticks to skip after a 429 before trying again.
const RATE_LIMIT_BACKOFF_TICKS: u32 = 4;

pub fn spawn_poll_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let mut backoff_ticks: u32 = 0;
        // Force a fetch on the very first tick.
        let mut last_fetch = Instant::now() - Duration::from_secs(24 * 60 * 60);

        loop {
            let state = app.state::<AppState>();
            let interval_secs = {
                state
                    .settings
                    .lock()
                    .unwrap()
                    .refresh_interval_secs
                    .max(MIN_REFRESH_INTERVAL_SECS)
            };
            let forced = {
                let mut f = state.force_refresh.lock().unwrap();
                std::mem::take(&mut *f)
            };

            let due = forced || last_fetch.elapsed().as_secs() >= interval_secs;

            if due {
                if backoff_ticks > 0 {
                    backoff_ticks -= 1;
                } else {
                    if let Err(UsageError::RateLimited) = do_fetch_once(&app, &client).await {
                        backoff_ticks = RATE_LIMIT_BACKOFF_TICKS;
                    }
                    last_fetch = Instant::now();
                }
            }

            // Tray/menu mutation must happen on the main thread — this
            // loop runs on a background tokio task, and calling into
            // AppKit (via muda's NSMenu bindings) from another thread
            // while the tray menu is open has been observed to panic.
            let app_for_main = app.clone();
            let _ = app.run_on_main_thread(move || {
                crate::tray::update_tray(&app_for_main);
            });
            emit_usage_updated(&app);

            let state = app.state::<AppState>();
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = state.notify.notified() => {}
            }
        }
    });
}

fn emit_usage_updated(app: &AppHandle) {
    let state = app.state::<AppState>();
    let dto = {
        let latest = state.latest.lock().unwrap();
        format::build_usage_dto(&latest)
    };
    let _ = app.emit("usage-updated", dto);
}

/// Resolves a token (via the cached credentials or a fresh blocking
/// Keychain/file read), fetches usage, retries once on 401 with a
/// forced credential re-read, and updates shared state with the
/// outcome. Returns the terminal error, if any, so the caller can
/// decide on backoff.
async fn do_fetch_once(app: &AppHandle, client: &reqwest::Client) -> Result<(), UsageError> {
    let token = match resolve_token(app, false).await {
        Ok(t) => t,
        Err(CredError::NotLoggedIn) => {
            set_status(app, FetchStatus::NotLoggedIn);
            return Ok(());
        }
        Err(CredError::KeychainDenied) => {
            set_status(app, FetchStatus::NotLoggedIn);
            return Ok(());
        }
        Err(CredError::ParseError(_)) => {
            set_status(app, FetchStatus::NotLoggedIn);
            return Ok(());
        }
    };

    match usage::fetch_usage(client, &token).await {
        Ok(resp) => {
            set_success(app, resp);
            Ok(())
        }
        Err(UsageError::Unauthorized) => match resolve_token(app, true).await {
            Ok(token2) => match usage::fetch_usage(client, &token2).await {
                Ok(resp) => {
                    set_success(app, resp);
                    Ok(())
                }
                Err(UsageError::Unauthorized) => {
                    set_status(app, FetchStatus::Unauthorized);
                    Ok(())
                }
                Err(e) => {
                    set_stale_if_possible(app);
                    Err(e)
                }
            },
            Err(_) => {
                set_status(app, FetchStatus::Unauthorized);
                Ok(())
            }
        },
        Err(e @ UsageError::RateLimited) => {
            set_stale_if_possible(app);
            Err(e)
        }
        Err(e) => {
            set_stale_if_possible(app);
            Err(e)
        }
    }
}

async fn resolve_token(app: &AppHandle, force: bool) -> Result<String, CredError> {
    let state = app.state::<AppState>();

    if !force {
        let cached = state.credentials.lock().unwrap().clone();
        if let Some(creds) = cached {
            if !creds.is_expired() {
                return Ok(creds.access_token);
            }
        }
    }

    let creds = tokio::task::spawn_blocking(credentials::read_credentials_blocking)
        .await
        .map_err(|e| CredError::ParseError(e.to_string()))??;

    let token = creds.access_token.clone();
    *state.credentials.lock().unwrap() = Some(creds);
    Ok(token)
}

fn set_success(app: &AppHandle, resp: usage::UsageResponse) {
    let state = app.state::<AppState>();
    let mut latest = state.latest.lock().unwrap();
    latest.response = Some(resp);
    latest.fetched_at = Some(std::time::SystemTime::now());
    latest.status = FetchStatus::Ok;
}

fn set_status(app: &AppHandle, status: FetchStatus) {
    let state = app.state::<AppState>();
    let mut latest = state.latest.lock().unwrap();
    latest.status = status;
}

/// On a transient failure (network/rate-limit), keep whatever data we
/// already have and just mark it stale — never blank the tray out.
fn set_stale_if_possible(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut latest = state.latest.lock().unwrap();
    if latest.response.is_some() {
        latest.status = FetchStatus::Stale;
    }
    // If we never had data, leave status as-is (NoDataYet) rather than
    // claiming staleness of data that never existed.
}
