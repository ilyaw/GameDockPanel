use tauri::{AppHandle, Manager};

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
