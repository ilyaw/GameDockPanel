//! Windows dock window setup, geometry, DWM corners, and Mica backdrop.

use std::mem::size_of;

use tauri::{App, Manager, WebviewWindow};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DWM_WINDOW_CORNER_PREFERENCE,
};

use crate::commands::apps::{AppsState, MenuOverlayState};
use crate::commands::settings::DockWindowLayer;
use crate::platform::geometry::{
    apply_dock_window_frame, current_dock_position, current_icon_size_dip,
    formula_window_frame_rest, resize_dock_window_for_pill, store_pill_dims,
};

use super::input::start_dock_input;

/// Re-applies DWM rounding + Mica after any native resize (menu overlay
/// grow/shrink, settings-driven geometry sync). Mica covers the full HWND —
/// resting frame is pill-centric; magnify margin may show glass during hover.
pub fn refresh_windows_backdrop(window: &WebviewWindow) -> Result<(), String> {
    apply_dwm_rounded_corners(window)?;
    apply_dock_mica(window)
}

fn apply_dwm_rounded_corners(window: &WebviewWindow) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let preference = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const DWM_WINDOW_CORNER_PREFERENCE as *const _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn apply_dock_mica(window: &WebviewWindow) -> Result<(), String> {
    use window_vibrancy::apply_mica;

    // Always-dark, like macOS HudWindow — not system semantic Mica.
    apply_mica(window, Some(true)).map_err(|e| e.to_string())
}

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
        formula_window_frame_rest(&entries, icon_size_dip, position);

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

    refresh_windows_backdrop(&window)?;

    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;

    store_pill_dims(&window, pill_width, pill_height);

    start_dock_input(window.clone());

    window.show().map_err(|e| e.to_string())?;
    log::info!("windows setup_dock_window: window shown");
    Ok(())
}

pub fn ensure_window_fits_menu_overlay(
    window: &WebviewWindow,
    overlay: MenuOverlayState,
) -> Result<(), String> {
    crate::platform::geometry::ensure_window_fits_menu_overlay(window, overlay)?;
    refresh_windows_backdrop(window)
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
    let changed = resize_dock_window_for_pill(window, pill_width, pill_height, icon_size_dip)?;
    if changed {
        refresh_windows_backdrop(window)?;
    }
    Ok(changed)
}

pub fn sync_vibrancy_pill_from_web(
    window: &WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let _ = (x, y);
    store_pill_dims(window, width, height);

    let icon_size_dip = current_icon_size_dip(window);
    let changed = resize_dock_window_for_pill(window, width, height, icon_size_dip)?;
    if changed {
        refresh_windows_backdrop(window)?;
    } else {
        apply_dock_mica(window)?;
    }

    log::debug!(
        "sync_vibrancy_pill (windows mica): w={width:.0} h={height:.0} resized={changed}"
    );
    Ok(())
}
