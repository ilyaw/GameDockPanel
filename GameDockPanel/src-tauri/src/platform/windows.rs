//! Windows dock window setup and geometry — no vibrancy/click-through yet.

use tauri::{App, Manager, WebviewWindow};

use crate::commands::apps::AppsState;
use crate::commands::settings::DockWindowLayer;
use crate::platform::geometry::{
    apply_dock_window_frame, current_dock_position, current_icon_size_dip, formula_window_frame,
    resize_dock_window_for_pill,
};

pub fn apply_dock_window_layer(
    window: &WebviewWindow,
    layer: DockWindowLayer,
) -> Result<(), String> {
    let on_top = matches!(layer, DockWindowLayer::AboveWindows);
    window
        .set_always_on_top(on_top)
        .map_err(|e| e.to_string())
}

/// Sizes the dock from the app roster, anchors it, and shows the window.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let icon_size_dip = current_icon_size_dip(&window);
    let position = current_dock_position(&window);
    let entries = app.state::<AppsState>().entries_snapshot();
    let (pill_width, pill_height, window_width, window_height) =
        formula_window_frame(&entries, icon_size_dip, position);

    apply_dock_window_frame(&window, window_width, window_height, position)?;

    let layer = {
        let state = app.state::<crate::commands::settings::SettingsState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.dock_window_layer
    };
    apply_dock_window_layer(&window, layer)?;

    {
        let state = app.state::<AppsState>();
        let mut current_width = state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_width = pill_width;
        let mut current_height = state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_height = pill_height;
    }

    window.show().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn shrink_dock_window_to_stored_pill(window: &WebviewWindow) -> Result<bool, String> {
    let state = window.state::<AppsState>();
    let pill_width = *state
        .pill_width_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let pill_height = *state
        .pill_height_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if pill_width < 1.0 || pill_height < 1.0 {
        return Ok(false);
    }
    let icon_size_dip = current_icon_size_dip(window);
    resize_dock_window_for_pill(window, pill_width, pill_height, icon_size_dip)
}
