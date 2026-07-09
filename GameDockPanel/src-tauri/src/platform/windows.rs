//! Minimal Windows dock window setup — show, bottom-center placement, and
//! always-on-top from persisted settings. Full geometry/vibrancy/click-through
//! remain macOS-only until a dedicated Windows pass.

use tauri::{App, Manager, PhysicalPosition};

use crate::commands::settings::{DockWindowLayer, SettingsState};

/// Shows the dock and anchors it to the bottom-center of the primary monitor.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let always_on_top = {
        let state = app.state::<SettingsState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.dock_window_layer == DockWindowLayer::AboveWindows
    };

    window
        .set_always_on_top(always_on_top)
        .map_err(|e| e.to_string())?;

    position_dock_bottom_center(&window)?;

    window.show().map_err(|e| e.to_string())?;
    Ok(())
}

fn position_dock_bottom_center(
    window: &tauri::WebviewWindow,
) -> Result<(), String> {
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "primary monitor not found".to_string())?;

    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();
    let window_size = window
        .outer_size()
        .map_err(|e| e.to_string())?;

    let x = monitor_pos.x
        + ((monitor_size.width as i32 - window_size.width as i32) / 2).max(0);
    let y = monitor_pos.y
        + (monitor_size.height as i32 - window_size.height as i32).max(0);

    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(())
}
