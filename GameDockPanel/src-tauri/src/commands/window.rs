use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::commands::apps::AppsState;
use crate::platform;

/// Aligns the native vibrancy blur view with the dock pill's actual DOM box.
/// CSS layout is the source of truth — Rust's width formula can drift from
/// flex rounding, borders, or list defaults.
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
/// menu's own measured `getBoundingClientRect().height`. See
/// `AppsState::menu_overlay_height_dip` for why the native click-through
/// hit-test needs this — a bare `Ok(())` here (rather than routing through
/// `platform::`) is enough since this only ever writes shared state, no
/// AppKit calls are involved.
#[tauri::command]
pub fn set_menu_overlay(state: State<AppsState>, active: bool, height: f64) -> Result<(), String> {
    let mut overlay_height = state
        .menu_overlay_height_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *overlay_height = if active { height.max(0.0) } else { 0.0 };
    Ok(())
}

/// Opens the settings window — shared by the tray icon and the dock's
/// settings button. Lazily creates the window on first call.
pub fn open_settings_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Ordinary framed utility window — not the dock's transparent/overlay
    // styling. Points at the same `index.html` bundle as the dock; `App.tsx`
    // picks the UI to render from `getCurrentWebviewWindow().label`.
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("GameDockPanel — Settings")
        .inner_size(460.0, 560.0)
        .min_inner_size(380.0, 440.0)
        .resizable(true)
        .decorations(true)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    open_settings_window(&app)
}
