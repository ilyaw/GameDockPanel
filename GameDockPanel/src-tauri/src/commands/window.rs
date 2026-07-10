use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::apps::{AppsState, MenuOverlaySide, MenuOverlayState};
use crate::platform;

/// Resizes the native window to fit the measured pill width/height (inner
/// size + magnify/glow margins), re-centering it in the same call. Call
/// before `sync_vibrancy_pill` when the pill size may have changed — then
/// re-measure the DOM after layout settles.
#[tauri::command]
pub fn resize_dock_window(
    app: AppHandle,
    pill_width: f64,
    pill_height: f64,
    icon_size_px: f64,
) -> Result<bool, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    platform::resize_dock_window_for_pill(&window, pill_width, pill_height, icon_size_px)
}

/// Aligns the native vibrancy blur mask to the pill's measured DOM box.
/// Does not resize the window — use `resize_dock_window` first when width
/// changed, then re-measure `getBoundingClientRect()` before calling this.
#[tauri::command]
pub fn sync_vibrancy_pill(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    platform::sync_vibrancy_pill_from_web(&window, x, y, width, height)
}

/// Called by `DockIcon` whenever its context menu opens or closes, with the
/// menu's measured footprint and resolved placement side. See
/// `AppsState::menu_overlay` for why the native click-through hit-test
/// needs this — a bare `Ok(())` here (rather than routing through
/// `platform::`) is enough since this only ever writes shared state, no
/// AppKit calls are involved.
#[tauri::command]
pub fn set_menu_overlay(
    app: AppHandle,
    state: State<AppsState>,
    active: bool,
    side: String,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let was_active = {
        let guard = state
            .menu_overlay
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.is_active()
    };

    let overlay = if active && (width > 0.0 || height > 0.0) {
        MenuOverlayState {
            side: MenuOverlaySide::parse(&side),
            width_dip: width.max(0.0),
            height_dip: height.max(0.0),
        }
    } else {
        MenuOverlayState::default()
    };

    {
        let mut guard = state
            .menu_overlay
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = overlay;
    }

    if let Some(window) = app.get_webview_window("main") {
        if overlay.is_active() {
            platform::ensure_window_fits_menu_overlay(&window, overlay)?;
        } else if was_active {
            platform::shrink_dock_window_to_stored_pill(&window)?;
            let _ = app.emit("dock-menu-overlay-closed", ());
        }
    }

    Ok(())
}

/// Opens the settings window — shared by the tray icon and tray menu.
/// The `settings` webview is pre-declared in `tauri.conf.json` (hidden at
/// startup) so we never call `WebviewWindowBuilder` from a sync handler —
/// on Windows that deadlocks WebView2 and leaves a blank white window.
pub fn open_settings_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| "settings window not found".to_string())?;
    // The dock window is always-on-top by default — without this the settings
    // webview can open behind it (looks like a broken tray click / "cached" UI).
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    window.center().map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    open_settings_window(&app)
}
