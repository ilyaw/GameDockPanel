//! Single owner of dock HWND chrome on Windows.
//!
//! **Law:** only this module (via [`ChromeGuard`]) may mutate `GWL_STYLE` /
//! `GWL_EXSTYLE`, WebView2 `DefaultBackgroundColor`, and force DWM redraws
//! after Tao/focus/size stomps. Region clips live in [`super::region`].
//!
//! Banned on the dock HWND path:
//! - `set_ignore_cursor_events` (restores caption / white ghost bar)
//! - Tao `set_size` / `set_position` (use native `SetWindowPos`)
//! - Mica / Acrylic
//! - `window.show()` outside [`super::lifecycle::show_main_window`]

use tauri::WebviewWindow;

use super::diag_file;
use super::window;

/// Facade over the Win32 chrome repair path in `window.rs`.
pub struct ChromeGuard;

impl ChromeGuard {
    /// Full prepare before first show: frameless popup, LAYERED, transparent
    /// WebView2 bg, clear Mica, install subclass.
    pub fn prepare(
        window: &WebviewWindow,
        window_width: f64,
        window_height: f64,
        position: crate::commands::settings::DockPosition,
    ) -> Result<(), String> {
        diag_file::status(
            "CHROME",
            "PREPARE",
            format!("frame={window_width:.1}x{window_height:.1} pos={position:?}"),
        );
        window::chrome_prepare(window, window_width, window_height, position)?;
        diag_file::ok("CHROME", "prepare complete");
        Ok(())
    }

    /// Re-assert frameless + LAYERED + transparent bg without changing formula
    /// size ownership (uses stored pill).
    pub fn reassert(window: &WebviewWindow) {
        diag_file::status("CHROME", "REASSERT", "keep_size");
        window::reassert_frameless_chrome_keep_size(window);
        window::chrome_invalidate(window);
    }

    /// Called on focus / size / DPI / post-launch — one path for surface changes.
    pub fn on_surface_changed(window: &WebviewWindow) {
        diag_file::status("CHROME", "SURFACE", "on_surface_changed");
        window::on_surface_changed(window);
    }

    /// Post-`show()` chrome repair with explicit frame size.
    pub fn reassert_after_show(
        window: &WebviewWindow,
        window_width: f64,
        window_height: f64,
        position: crate::commands::settings::DockPosition,
    ) {
        diag_file::status(
            "CHROME",
            "AFTER_SHOW",
            format!("frame={window_width:.1}x{window_height:.1}"),
        );
        window::chrome_reassert_after_show(window, window_width, window_height, position);
    }
}
