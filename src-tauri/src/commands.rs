use tauri::{AppHandle, Manager, State};

use crate::format::{self, UsageDto};
use crate::settings::{self, Settings};
use crate::state::AppState;

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Settings {
    let clamped = settings.clamped();
    *state.settings.lock().unwrap() = clamped.clone();
    settings::save_settings_to_disk(&app, &clamped);

    // Wake the poll loop so the tray reflects visibility changes (and,
    // if the interval was shortened, potentially fetches sooner) right
    // away rather than waiting for the next 30s tick.
    state.notify.notify_one();

    // Tray/menu mutation must happen on the main thread (see poll.rs).
    let app_for_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        crate::tray::update_tray(&app_for_main);
    });

    clamped
}

#[tauri::command]
pub fn refresh_now(app: AppHandle) {
    request_refresh_now(app);
}

/// Shared by the `refresh_now` command and the tray menu's "Refresh
/// Now" item, so both entry points behave identically.
pub fn request_refresh_now(app: AppHandle) {
    let state = app.state::<AppState>();
    *state.force_refresh.lock().unwrap() = true;
    state.notify.notify_one();
}

#[tauri::command]
pub fn get_latest_usage(state: State<AppState>) -> UsageDto {
    let latest = state.latest.lock().unwrap();
    format::build_usage_dto(&latest)
}
