use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Polling any faster than this trips the endpoint's rate limiting.
pub const MIN_REFRESH_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub show_session: bool,
    pub show_weekly: bool,
    pub show_fable: bool,
    pub refresh_interval_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_session: true,
            show_weekly: true,
            show_fable: true,
            refresh_interval_secs: 60,
        }
    }
}

impl Settings {
    pub fn clamped(mut self) -> Self {
        if self.refresh_interval_secs < MIN_REFRESH_INTERVAL_SECS {
            self.refresh_interval_secs = MIN_REFRESH_INTERVAL_SECS;
        }
        self
    }
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("settings.json"))
}

pub fn load_settings(app: &AppHandle) -> Settings {
    config_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
        .unwrap_or_default()
        .clamped()
}

pub fn save_settings_to_disk(app: &AppHandle, settings: &Settings) {
    if let Some(path) = config_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(settings) {
            let _ = std::fs::write(path, json);
        }
    }
}
