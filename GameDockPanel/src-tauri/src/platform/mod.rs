//! Platform-specific window setup, kept out of `lib.rs` (see dockpanel rule:
//! "Platform-specific код — только в `platform/`").

/// Result of resolving a native app icon — PNG data URL plus an optional
/// accent color sampled from the same bitmap.
#[derive(Clone, Debug, Default)]
pub struct IconResolveResult {
    pub icon_url: Option<String>,
    pub accent_color: Option<String>,
}

mod icon_accent;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod geometry;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{
    activate_or_launch_app, apply_dock_window_layer, is_app_installed, is_bundle_running,
    quit_app, refresh_dock_icons, resolve_bundle_id_from_path, resolve_app_icon,
    reveal_app_in_finder, resize_dock_window_for_pill, resize_dock_window_for_app_count,
    setup_dock_window, start_apps_monitoring, sync_vibrancy_pill_from_web,
    ensure_window_fits_menu_overlay, shrink_dock_window_to_stored_pill,
    zoom_app_above_dock,
};

#[cfg(target_os = "windows")]
pub(crate) use windows::seed::seed_app_candidates;

#[cfg(target_os = "windows")]
pub use windows::{
    activate_or_launch_app, apply_dock_window_layer, clear_dock_menu_region_hold,
    ensure_window_fits_menu_overlay, is_bundle_running, log_windows_diag_snapshot, quit_app,
    refresh_dock_icons, resolve_bundle_id_from_path, resolve_app_icon, reveal_app_in_finder,
    set_dock_region_relaxed, setup_dock_window, shrink_dock_window_to_stored_pill,
    start_apps_monitoring, store_frontend_render_metrics, sync_vibrancy_pill_from_web,
    windows_backdrop_snapshot, zoom_app_above_dock,
};

#[cfg(target_os = "windows")]
pub use geometry::resize_dock_window_for_pill;

/// Non-macOS / non-Windows stubs.
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn apply_dock_window_layer(
    _window: &tauri::WebviewWindow,
    _layer: crate::commands::settings::DockWindowLayer,
) -> Result<(), String> {
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn setup_dock_window(app: &mut tauri::App) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn start_apps_monitoring(_app: &tauri::App) -> Result<(), String> {
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn activate_or_launch_app(_app: tauri::AppHandle, _bundle_id: String) -> Result<(), String> {
    Err("process monitoring is not implemented on this platform yet".to_string())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn is_app_installed(_bundle_id: &str) -> bool {
    false
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn is_bundle_running(_bundle_id: &str) -> bool {
    false
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn resolve_bundle_id_from_path(_path: &str) -> Result<String, String> {
    Err("adding apps is not implemented on this platform yet".to_string())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn resolve_app_icon(
    _bundle_id: &str,
    _icon_size_dip: f64,
    _scale_factor: f64,
) -> IconResolveResult {
    IconResolveResult::default()
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn refresh_dock_icons(_app: &tauri::AppHandle, _state: &crate::commands::apps::AppsState) {}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn resize_dock_window_for_pill(
    _window: &tauri::WebviewWindow,
    _pill_width: f64,
    _pill_height: f64,
    _icon_size_dip: f64,
) -> Result<bool, String> {
    Ok(false)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn resize_dock_window_for_app_count(
    _window: &tauri::WebviewWindow,
    _entries: &[crate::commands::apps::DockItem],
) -> Result<bool, String> {
    Ok(false)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn ensure_window_fits_menu_overlay(
    _window: &tauri::WebviewWindow,
    _overlay: crate::commands::apps::MenuOverlayState,
) -> Result<(), String> {
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn shrink_dock_window_to_stored_pill(_window: &tauri::WebviewWindow) -> Result<bool, String> {
    Ok(false)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn sync_vibrancy_pill_from_web(
    _window: &tauri::WebviewWindow,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
) -> Result<(), String> {
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn quit_app(_app: tauri::AppHandle, _bundle_id: String) -> Result<(), String> {
    Err("process monitoring is not implemented on this platform yet".to_string())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn reveal_app_in_finder(_app: tauri::AppHandle, _bundle_id: String) -> Result<(), String> {
    Err("process monitoring is not implemented on this platform yet".to_string())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn zoom_app_above_dock(_app: tauri::AppHandle, _bundle_id: String) -> Result<(), String> {
    Err("dock zoom is not implemented on this platform yet".to_string())
}
