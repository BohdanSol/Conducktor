//! Single place where usage numbers become human-readable strings.
//! Both the tray title/menu (Rust) and the Settings window (TS, via the
//! `get_latest_usage` command / `usage-updated` event) consume the
//! `UsageDto` this module produces, so formatting never drifts between
//! the two surfaces.

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::settings::Settings;
use crate::state::{FetchStatus, LatestUsage};
use crate::usage::Bucket;

pub fn format_pct(utilization: f64) -> String {
    format!("{}%", utilization.round() as i64)
}

/// `detailed = false` matches the tray-title spec: minutes under 1h,
/// hours-only between 1h and 24h, `Xd Yh` beyond that.
/// `detailed = true` adds minutes to the 1h–24h bucket for the fuller
/// menu/Settings display (e.g. "4h 55m" instead of just "4h").
pub fn format_countdown(resets_at_secs: i64, now: SystemTime, detailed: bool) -> String {
    let now_secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = (resets_at_secs - now_secs).max(0);

    let days = delta / 86_400;
    let hours = (delta % 86_400) / 3_600;
    let mins = (delta % 3_600) / 60;

    if days >= 1 {
        format!("{days}d {hours}h")
    } else if delta >= 3_600 {
        if detailed {
            format!("{hours}h {mins}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{}m", mins.max(1))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MeterDto {
    pub utilization: f64,
    pub pct_label: String,
    /// Detailed form ("4h 55m") for the Settings window / tray menu.
    /// `None` for the Fable meter, which always resets alongside Weekly.
    pub reset_label: Option<String>,
    /// Raw epoch seconds, so the tray title can independently render the
    /// terser "in 5h" form without re-parsing `reset_label`.
    pub resets_at_epoch_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageDto {
    pub session: Option<MeterDto>,
    pub weekly: Option<MeterDto>,
    pub weekly_fable: Option<MeterDto>,
    /// Unix seconds of the last successful fetch, if any.
    pub fetched_at_epoch_secs: Option<i64>,
    pub status: String,
    pub status_message: String,
}

fn meter_dto(bucket: &Bucket, now: SystemTime, show_reset: bool) -> MeterDto {
    let resets_at = if show_reset { bucket.resets_at } else { None };
    MeterDto {
        utilization: bucket.utilization,
        pct_label: format_pct(bucket.utilization),
        reset_label: resets_at.map(|r| format_countdown(r, now, true)),
        resets_at_epoch_secs: resets_at,
    }
}

pub fn status_str(status: FetchStatus) -> &'static str {
    match status {
        FetchStatus::Ok => "ok",
        FetchStatus::Stale => "stale",
        FetchStatus::NotLoggedIn => "not_logged_in",
        FetchStatus::Unauthorized => "unauthorized",
        FetchStatus::NoDataYet => "no_data_yet",
    }
}

pub fn status_message(status: FetchStatus) -> &'static str {
    match status {
        FetchStatus::Ok => "Up to date",
        FetchStatus::Stale => "Last refresh failed — showing previous values",
        FetchStatus::NotLoggedIn => {
            "Not logged in to Claude Code — run `claude` in a terminal and sign in, then Refresh"
        }
        FetchStatus::Unauthorized => "Claude Code login was rejected — try logging in again",
        FetchStatus::NoDataYet => "Fetching…",
    }
}

pub fn build_usage_dto(latest: &LatestUsage) -> UsageDto {
    let now = SystemTime::now();
    let response = latest.response.as_ref();

    UsageDto {
        session: response
            .and_then(|r| r.five_hour.as_ref())
            .map(|b| meter_dto(b, now, true)),
        weekly: response
            .and_then(|r| r.seven_day.as_ref())
            .map(|b| meter_dto(b, now, true)),
        weekly_fable: response
            .and_then(|r| r.seven_day_fable.as_ref())
            .map(|b| meter_dto(b, now, false)),
        fetched_at_epoch_secs: latest.fetched_at.and_then(|t| {
            t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
        }),
        status: status_str(latest.status).to_string(),
        status_message: status_message(latest.status).to_string(),
    }
}

/// Tray-bar title, e.g. "S 5% (in 5h) · W 2% (in 3d 20h) · F 1%".
pub fn build_tray_title(dto: &UsageDto, settings: &Settings) -> String {
    match dto.status.as_str() {
        "not_logged_in" => return "Claude: login required".to_string(),
        "unauthorized" => return "Claude: login rejected".to_string(),
        "no_data_yet" if dto.session.is_none() && dto.weekly.is_none() && dto.weekly_fable.is_none() => {
            return "Claude: …".to_string()
        }
        _ => {}
    }

    let now = SystemTime::now();
    let short_reset = |m: &MeterDto| m.resets_at_epoch_secs.map(|r| format_countdown(r, now, false));

    let mut segments = Vec::new();

    if settings.show_session {
        if let Some(m) = &dto.session {
            segments.push(match short_reset(m) {
                Some(reset) => format!("S {} (in {})", m.pct_label, reset),
                None => format!("S {}", m.pct_label),
            });
        }
    }
    if settings.show_weekly {
        if let Some(m) = &dto.weekly {
            segments.push(match short_reset(m) {
                Some(reset) => format!("W {} (in {})", m.pct_label, reset),
                None => format!("W {}", m.pct_label),
            });
        }
    }
    if settings.show_fable {
        if let Some(m) = &dto.weekly_fable {
            segments.push(format!("F {}", m.pct_label));
        }
    }

    if segments.is_empty() {
        return "Claude: …".to_string();
    }

    let mut title = segments.join(" · ");
    if dto.status == "stale" {
        title.push_str(" ⚠");
    }
    title
}
