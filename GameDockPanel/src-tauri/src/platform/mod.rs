//! Platform-specific window setup, kept out of `lib.rs` (see dockpanel rule:
//! "Platform-specific код — только в `platform/`").

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::setup_dock_window;

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
