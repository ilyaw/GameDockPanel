use tauri::{AppHandle, Manager, State};

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
