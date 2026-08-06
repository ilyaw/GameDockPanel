//! Dock window lifecycle: hidden prepare → frontend ready → show once.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use tauri::{App, AppHandle, Manager};

use crate::commands::apps::AppsState;
use crate::platform::geometry::{
    apply_dock_window_frame, current_dock_position, current_icon_size_dip, formula_window_frame_rest,
    store_pill_dims,
};

use super::chrome::ChromeGuard;
use super::diag_file;
use super::input::start_dock_input;
use super::region;
use super::window::{
    apply_dock_window_layer, fallback_pill_for_setup, log_display_snapshot,
    reassert_dwm_alpha_for_window, set_dock_click_through, start_windows_diag_poller,
    store_pill_client_rect_for_setup, windows_backdrop_snapshot,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShowPhase {
    Idle,
    Showing,
    Shown,
}

struct ShowGate {
    phase: Mutex<ShowPhase>,
    cv: Condvar,
}

impl ShowGate {
    const fn new() -> Self {
        Self {
            phase: Mutex::new(ShowPhase::Idle),
            cv: Condvar::new(),
        }
    }
}

static SHOW_GATE: ShowGate = ShowGate::new();

/// Prepare chrome + region while keeping `visible: false`. Does **not** show.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    diag_file::init(app.handle())?;

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

    let scale = window.scale_factor().unwrap_or(1.0);
    diag_file::status(
        "SETUP",
        "BEGIN",
        format!(
            "position={position:?} icon={icon_size_dip} apps={entry_count} \
             pill={pill_width:.1}x{pill_height:.1} window={window_width:.1}x{window_height:.1} \
             scale={scale:.2}"
        ),
    );
    log::info!(
        "[win-backdrop] setup_dock_window (hidden): position={position:?} icon_size={icon_size_dip} \
         apps={entry_count} pill={pill_width:.1}x{pill_height:.1} \
         window={window_width:.1}x{window_height:.1} scale={scale:.2}"
    );

    // Transparent WebView2 bg before any paint / show.
    if let Err(err) = window.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0))) {
        diag_file::status("CHROME", "BG_FAIL", format!("{err}"));
        log::warn!("[win-backdrop] set_background_color(transparent) at setup: {err}");
    } else {
        diag_file::ok("CHROME", "backgroundColor=#00000000");
    }

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

    ChromeGuard::prepare(&window, window_width, window_height, position)?;

    set_dock_click_through(&window, true)?;

    store_pill_dims(&window, pill_width, pill_height);
    if let Ok(rect) = fallback_pill_for_setup(&window, pill_width, pill_height) {
        store_pill_client_rect_for_setup(rect.0, rect.1, rect.2, rect.3);
        diag_file::ok(
            "SETUP",
            format!(
                "fallback_pill=({:.1},{:.1} {:.1}x{:.1})",
                rect.0, rect.1, rect.2, rect.3
            ),
        );
    }

    // Best-effort — a failed region must not abort dock setup (same as pre-show).
    if let Err(err) = region::refresh(&window) {
        diag_file::status("SETUP", "RGN_WARN", &err);
        log::warn!("[win-backdrop] setup region refresh failed (continuing): {err}");
    }

    start_dock_input(window.clone());
    start_windows_diag_poller(window.clone());

    diag_file::ok(
        "SETUP",
        format!(
            "hidden prepared snapshot={:?}",
            windows_backdrop_snapshot(&window)
        ),
    );
    log::info!(
        "[win-backdrop] setup_dock_window: hidden prepared (awaiting show_main_window) snapshot={:?}",
        windows_backdrop_snapshot(&window)
    );
    Ok(())
}

/// Frontend calls this after React + geometry sync.
///
/// Concurrent invokes share one in-flight show (`Showing`):
/// - waiters block on the condvar until the leader finishes;
/// - on success all see `Shown` and return `Ok` (window is visible);
/// - on failure phase returns to `Idle` and a waiter may become the next
///   leader and retry `perform_show` in the same invoke.
pub fn show_main_window(app: &AppHandle) -> Result<(), String> {
    // Claim leadership or wait for the in-flight / completed show.
    {
        let mut phase = SHOW_GATE
            .phase
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        loop {
            match *phase {
                ShowPhase::Shown => {
                    diag_file::status("SHOW", "SKIP", "already shown");
                    return Ok(());
                }
                ShowPhase::Showing => {
                    diag_file::status("SHOW", "WAIT", "in-flight show");
                    let (guard, wait_result) = SHOW_GATE
                        .cv
                        .wait_timeout(phase, Duration::from_secs(15))
                        .unwrap_or_else(|p| p.into_inner());
                    phase = guard;
                    if wait_result.timed_out() && *phase == ShowPhase::Showing {
                        diag_file::status("SHOW", "ERR", "wait for in-flight show timed out");
                        return Err("show_main_window: timed out waiting for in-flight show".into());
                    }
                    // Loop: re-check Shown / Idle / still Showing.
                }
                ShowPhase::Idle => {
                    *phase = ShowPhase::Showing;
                    break;
                }
            }
        }
    }

    let result = perform_show(app);

    {
        let mut phase = SHOW_GATE
            .phase
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match &result {
            Ok(()) => {
                *phase = ShowPhase::Shown;
            }
            Err(err) => {
                *phase = ShowPhase::Idle;
                diag_file::status("SHOW", "ERR", format!("rolled back: {err}"));
            }
        }
    }
    SHOW_GATE.cv.notify_all();
    result
}

fn perform_show(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    diag_file::status("SHOW", "BEGIN", "frontend ready gate");

    if let Err(err) = window.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0))) {
        diag_file::status("SHOW", "BG_WARN", format!("{err}"));
    }

    let icon_size_dip = current_icon_size_dip(&window);
    let position = current_dock_position(&window);
    let entries = app.state::<AppsState>().entries_snapshot();
    let (_pill_w, _pill_h, window_width, window_height) =
        formula_window_frame_rest(&entries, icon_size_dip, position);

    ChromeGuard::reassert(&window);
    // Pre-show region sync is best-effort — never block first paint.
    if let Err(err) = region::refresh(&window) {
        diag_file::status("SHOW", "RGN_WARN", format!("pre-show: {err}"));
        log::warn!("[win-backdrop] pre-show region refresh failed (continuing): {err}");
    }

    window.show().map_err(|e| e.to_string())?;
    diag_file::ok("SHOW", "window.show() ok");
    // First map can reset DWM state — pin per-pixel alpha before repaint.
    reassert_dwm_alpha_for_window(&window);

    ChromeGuard::reassert_after_show(&window, window_width, window_height, position);
    if let Err(err) = set_dock_click_through(&window, true) {
        diag_file::status("SHOW", "CLICK_THROUGH_WARN", &err);
        log::warn!("[win-backdrop] click-through after show failed: {err}");
    }
    if let Err(err) = region::refresh(&window) {
        diag_file::status("SHOW", "RGN_WARN", &err);
        log::warn!("[win-backdrop] pill region after show failed: {err}");
    }
    ChromeGuard::on_surface_changed(&window);

    // Icons may have been resolved while the window was still hidden / scale
    // unknown — re-rasterize at the live DPI once the HWND is mapped.
    {
        let state = app.state::<AppsState>();
        crate::platform::refresh_dock_icons(app, &state);
        log::info!("[icon-export] refreshed after show_main_window");
    }

    log_display_snapshot(&window);

    diag_file::ok(
        "SHOW",
        format!("complete snapshot={:?}", windows_backdrop_snapshot(&window)),
    );
    log::info!(
        "[win-backdrop] show_main_window: shown snapshot={:?}",
        windows_backdrop_snapshot(&window)
    );
    Ok(())
}
