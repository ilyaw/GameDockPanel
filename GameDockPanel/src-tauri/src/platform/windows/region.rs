//! Pill-shaped `SetWindowRgn` ownership.
//!
//! Always on for Windows — GDI RoundRect confines pale WebView2 crescents.
//! Soft CSS-only (no region) was removed after repeated white-corner
//! regressions. Never fully clear the region on the dock HWND. All clip
//! updates go through [`refresh`] / [`super::window`] region helpers.

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
