use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::format;
use crate::state::AppState;

pub const TRAY_ID: &str = "main";

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/32x32.png");

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let icon = Image::from_bytes(TRAY_ICON_BYTES)?;

    let refresh_item = MenuItemBuilder::with_id("refresh_now", "Refresh Now").build(app)?;
    let settings_item = MenuItemBuilder::with_id("open_settings", "Settings…").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let sep = PredefinedMenuItem::separator(app)?;

    let initial_menu = MenuBuilder::new(app)
        .item(&refresh_item)
        .item(&settings_item)
        .item(&sep)
        .item(&quit_item)
        .build()?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .title("Claude: …")
        .menu(&initial_menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "refresh_now" => crate::commands::request_refresh_now(app.clone()),
            "open_settings" => show_settings_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

pub fn show_settings_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Rebuilds the tray title and the detail lines in its dropdown menu
/// from the current shared state. Cheap enough to call on every poll
/// tick (including the between-fetch ticks that only refresh
/// countdowns).
pub fn update_tray(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let dto = {
        let latest = state.latest.lock().unwrap();
        format::build_usage_dto(&latest)
    };

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    let segments = format::build_tray_segments(&dto, &settings);

    // Baseline plain-text title: accessibility/VoiceOver text, and what
    // shows if the native rich-text path below can't find the button
    // (e.g. very early during startup).
    let title = format::build_tray_title(&dto, &settings);
    let _ = tray.set_title(Some(title));

    #[cfg(target_os = "macos")]
    apply_attributed_title(&tray, segments);

    if let Ok(menu) = build_detail_menu(app, &dto) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// Renders the tray title as a rich `NSAttributedString` so the "S"/"W"/"F"
/// labels and the "(in ...)" timing can be bold/small/gray while the
/// percentage values stay full-size and full-contrast. `NSStatusItem` only
/// exposes a plain string through Tauri's public API, so this reaches past
/// it to the underlying `NSStatusBarButton` via `with_inner_tray_icon`.
#[cfg(target_os = "macos")]
fn apply_attributed_title(tray: &tauri::tray::TrayIcon, segments: Vec<format::TraySegment>) {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName};
    use objc2_foundation::{NSAttributedString, NSDictionary, NSMutableAttributedString, NSString};

    let _ = tray.with_inner_tray_icon(move |inner| {
        let Some(status_item) = inner.ns_status_item() else {
            return;
        };
        // SAFETY: `with_inner_tray_icon` guarantees this closure runs on
        // the main thread (it dispatches via `run_item_main_thread!`).
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let Some(button) = status_item.button(mtm) else {
            return;
        };

        let label_font = NSFont::boldSystemFontOfSize(11.0);
        let value_font = NSFont::systemFontOfSize(13.0);
        let timing_font = NSFont::systemFontOfSize(11.0);
        let gray = NSColor::secondaryLabelColor();
        let primary = NSColor::labelColor();

        let combined = NSMutableAttributedString::new();

        for segment in &segments {
            let (font, color) = match segment.kind {
                format::TraySegmentKind::Label => (label_font.clone(), gray.clone()),
                format::TraySegmentKind::Value => (value_font.clone(), primary.clone()),
                format::TraySegmentKind::Timing | format::TraySegmentKind::Separator => {
                    (timing_font.clone(), gray.clone())
                }
            };

            // SAFETY: `NSFontAttributeName`/`NSForegroundColorAttributeName`
            // are non-null AppKit constants; reading an `extern "C" static`
            // is what `unsafe` guards here, not the values themselves.
            let keys: Vec<&NSString> = vec![
                unsafe { NSFontAttributeName },
                unsafe { NSForegroundColorAttributeName },
            ];
            let objects: Vec<Retained<AnyObject>> = vec![
                Retained::into_super(Retained::into_super(font)),
                Retained::into_super(Retained::into_super(color)),
            ];
            let attrs = NSDictionary::from_retained_objects(&keys, &objects);

            let ns_text = NSString::from_str(&segment.text);
            // SAFETY: `attrs`' generic key/value types match what
            // `initWithString:attributes:` expects (`NSAttributedStringKey`
            // = `NSString`, values type-erased to `AnyObject`).
            let attr_run = unsafe {
                NSAttributedString::initWithString_attributes(
                    NSAttributedString::alloc(),
                    &ns_text,
                    Some(&attrs),
                )
            };
            combined.appendAttributedString(&attr_run);
        }

        button.setAttributedTitle(&combined);
    });
}

fn build_detail_menu(
    app: &AppHandle,
    dto: &format::UsageDto,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let mut builder = MenuBuilder::new(app);

    let mut any_detail = false;

    if let Some(m) = &dto.session {
        let text = match &m.reset_label {
            Some(reset) => format!("Session: {} — resets in {}", m.pct_label, reset),
            None => format!("Session: {}", m.pct_label),
        };
        builder = builder.item(&MenuItemBuilder::new(text).enabled(false).build(app)?);
        any_detail = true;
    }
    if let Some(m) = &dto.weekly {
        let text = match &m.reset_label {
            Some(reset) => format!("Weekly: {} — resets in {}", m.pct_label, reset),
            None => format!("Weekly: {}", m.pct_label),
        };
        builder = builder.item(&MenuItemBuilder::new(text).enabled(false).build(app)?);
        any_detail = true;
    }
    if let Some(m) = &dto.weekly_fable {
        let text = format!("Weekly (Fable): {}", m.pct_label);
        builder = builder.item(&MenuItemBuilder::new(text).enabled(false).build(app)?);
        any_detail = true;
    }

    if !any_detail {
        builder = builder
            .item(&MenuItemBuilder::new(dto.status_message.clone()).enabled(false).build(app)?);
        any_detail = true;
    }

    if any_detail {
        let last_updated = dto
            .fetched_at_epoch_secs
            .map(format_last_updated)
            .unwrap_or_else(|| "Last updated: never".to_string());
        builder = builder.item(&MenuItemBuilder::new(last_updated).enabled(false).build(app)?);
        builder = builder.item(&PredefinedMenuItem::separator(app)?);
    }

    let refresh_item = MenuItemBuilder::with_id("refresh_now", "Refresh Now").build(app)?;
    let settings_item = MenuItemBuilder::with_id("open_settings", "Settings…").build(app)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    builder
        .item(&refresh_item)
        .item(&settings_item)
        .item(&sep)
        .item(&quit_item)
        .build()
}

fn format_last_updated(epoch_secs: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(epoch_secs, 0).single() {
        Some(dt) => format!("Last updated: {}", dt.format("%H:%M")),
        None => "Last updated: unknown".to_string(),
    }
}
