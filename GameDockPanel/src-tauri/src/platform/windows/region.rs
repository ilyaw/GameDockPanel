//! Pill-shaped `SetWindowRgn` ownership.
//!
//! Applies only when the `windowsHardClip` setting is on (default off — CSS
//! per-pixel alpha rounds the corners without GDI). In hard-clip mode, never
//! fully clear the region on the dock HWND — that re-opens pale WebView2
//! corners outside the CSS pill. All clip updates go through
//! [`refresh`] / [`super::window`] region helpers.

use tauri::WebviewWindow;

use super::diag_file;
use super::window;

/// Sync GDI RoundRect clip to the current CSS pill (or pill∪margins).
pub fn refresh(window: &WebviewWindow) -> Result<(), String> {
    diag_file::status("RGN", "REFRESH", "begin");
    match window::refresh_windows_backdrop(window) {
        Ok(()) => {
            diag_file::ok("RGN", "refresh ok");
            Ok(())
        }
        Err(err) => {
            diag_file::status("RGN", "ERR", &err);
            Err(err)
        }
    }
}
