//! Platform-specific window setup, kept out of `lib.rs` (see dockpanel rule:
//! "Platform-specific код — только в `platform/`").

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{
    activate_or_launch_app, is_app_installed, is_bundle_running, quit_app,
    resolve_bundle_id_from_path, resolve_icon_data_url as resolve_app_icon,
    reveal_app_in_finder, resize_dock_window_for_pill, setup_dock_window,
    start_apps_monitoring, sync_vibrancy_pill_from_web,
};

/// Windows/Linux support isn't implemented yet — no-op for now rather than
/// a placeholder `windows.rs` stub with nothing in it. Add that module (and
/// wire it up here) when cross-platform work actually starts.
#[cfg(not(target_os = "macos"))]
pub fn setup_dock_window(app: &mut tauri::App) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn start_apps_monitoring(_app: &tauri::App) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn activate_or_launch_app(_app: tauri::AppHandle, _bundle_id: String) -> Result<(), String> {
    Err("process monitoring is not implemented on this platform yet".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn is_app_installed(_bundle_id: &str) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn is_bundle_running(_bundle_id: &str) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn resolve_bundle_id_from_path(_path: &str) -> Result<String, String> {
    Err("adding apps is not implemented on this platform yet".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn resolve_app_icon(_bundle_id: &str) -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn resize_dock_window_for_pill(
    _window: &tauri::WebviewWindow,
    _pill_width: f64,
) -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(target_os = "macos"))]
pub fn sync_vibrancy_pill_from_web(
    _window: &tauri::WebviewWindow,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn quit_app(_app: tauri::AppHandle, _bundle_id: String) -> Result<(), String> {
    Err("process monitoring is not implemented on this platform yet".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn reveal_app_in_finder(_app: tauri::AppHandle, _bundle_id: String) -> Result<(), String> {
    Err("process monitoring is not implemented on this platform yet".to_string())
}
