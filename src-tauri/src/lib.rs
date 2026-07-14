mod commands;
mod credentials;
mod format;
mod poll;
mod settings;
mod state;
mod tray;
mod usage;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state::AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::refresh_now,
            commands::get_latest_usage,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();

            {
                let loaded = settings::load_settings(&handle);
                let state = handle.state::<state::AppState>();
                *state.settings.lock().unwrap() = loaded;
            }

            tray::build_tray(&handle)?;

            // The Settings window is created hidden; closing it should
            // just hide it again, not tear down the webview (and
            // definitely not quit the app — that's the tray's job).
            if let Some(win) = app.get_webview_window("settings") {
                let win_for_handler = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_for_handler.hide();
                    }
                });
            }

            poll::spawn_poll_loop(handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
