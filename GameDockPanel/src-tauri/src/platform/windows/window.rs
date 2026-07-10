//! Windows dock window setup, geometry, and menu-overlay sizing.

use tauri::{App, Manager, WebviewWindow};

use crate::commands::apps::AppsState;
use crate::commands::settings::DockWindowLayer;
use crate::platform::geometry::{
    apply_dock_window_frame, current_dock_position, current_icon_size_dip, formula_window_frame,
    resize_dock_window_for_pill, store_pill_dims,
};

use super::input::start_dock_input;

pub use crate::platform::geometry::ensure_window_fits_menu_overlay;

pub fn apply_dock_window_layer(
    window: &WebviewWindow,
    layer: DockWindowLayer,
) -> Result<(), String> {
    let on_top = matches!(layer, DockWindowLayer::AboveWindows);
    window
        .set_always_on_top(on_top)
        .map_err(|e| e.to_string())
}

/// Sizes the dock from the app roster, anchors it, enables click-through, and shows.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let icon_size_dip = current_icon_size_dip(&window);
    let position = current_dock_position(&window);
    let entries = app.state::<AppsState>().entries_snapshot();
    let entry_count = entries
        .iter()
        .filter(|item| matches!(item, crate::commands::apps::DockItem::App(_)))
        .count();

    let (pill_width, pill_height, window_width, window_height) =
        formula_window_frame(&entries, icon_size_dip, position);

    log::info!(
        "windows setup_dock_window: position={position:?} icon_size={icon_size_dip} \
         apps={entry_count} pill={pill_width:.0}x{pill_height:.0} window={window_width:.0}x{window_height:.0}"
    );

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

    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;

    store_pill_dims(&window, pill_width, pill_height);

    start_dock_input(window.clone());

    window.show().map_err(|e| e.to_string())?;
    log::info!("windows setup_dock_window: window shown");
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

pub fn sync_vibrancy_pill_from_web(
    window: &WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    store_pill_dims(window, width, height);
    log::debug!(
        "sync_vibrancy_pill (windows no-op blur): x={x:.0} y={y:.0} w={width:.0} h={height:.0}"
    );
    Ok(())
}
