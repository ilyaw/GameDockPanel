//! Windows dock window setup, geometry, DWM corners, and transparent backdrop.
//!
//! Win11 system Mica paints the full HWND and does not reliably respect a
//! GDI `SetWindowRgn` clip — that left dark corners outside the CSS/RGB
//! pill. We never apply Mica; tint is CSS (`bg-black/40`) over a transparent
//! WebView2. Idle still clips the HWND to the rounded pill as shape
//! insurance; hover/menu clear the region and keep `DONOTROUND` so expanded
//! margins stay transparent (not a dark rounded HWND shell).

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use tauri::utils::config::Color;
use tauri::{App, Manager, WebviewWindow};
use windows::Win32::Foundation::GetLastError;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};

use crate::commands::apps::{AppsState, MenuOverlayState};
use crate::commands::settings::{DockPosition, DockWindowLayer};
use crate::platform::geometry::{
    apply_dock_window_frame, current_dock_position, current_icon_size_dip, expand_for_hover,
    formula_window_frame_rest, resize_dock_window_for_pill, set_expand_for_hover,
    store_pill_dims, DOCK_EDGE_INSET_DIP, PILL_CORNER_RADIUS_DIP,
};

use super::input::start_dock_input;

/// Last CSS-pill box from `sync_vibrancy_pill` (DIP, window-client coords).
/// Used by `refresh_windows_backdrop` after resizes that don't re-measure.
#[derive(Clone, Copy, Debug)]
struct PillClientRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Snapshot for `get_diagnostics` — friend can paste this with the log file.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsBackdropSnapshot {
    pub last_pill_client: Option<WindowsPillRect>,
    pub menu_overlay_active: bool,
    pub region_relaxed: bool,
    pub menu_region_hold: bool,
    pub scale_factor: Option<f64>,
    pub inner_size_px: Option<(u32, u32)>,
    pub outer_size_px: Option<(u32, u32)>,
    pub outer_position_px: Option<(i32, i32)>,
    pub stored_pill_dip: Option<(f64, f64)>,
    pub dock_position: String,
    pub sync_vibrancy_calls: u64,
    pub set_rgn_ok_count: u64,
    pub set_rgn_err_count: u64,
    /// Raw `GWL_STYLE` (hex-friendly u32).
    pub gwl_style: Option<u32>,
    pub gwl_exstyle: Option<u32>,
    /// `outer − inner` size in px — frameless popup should be ~(0, 0).
    pub chrome_delta_px: Option<(i32, i32)>,
    /// Always `false` — Mica is disabled (dark full-HWND shell outside RGB).
    pub mica_enabled: bool,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsPillRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

static LAST_PILL_CLIENT: Mutex<Option<PillClientRect>> = Mutex::new(None);
static SYNC_VIBRANCY_CALLS: AtomicU64 = AtomicU64::new(0);
static SET_RGN_OK: AtomicU64 = AtomicU64::new(0);
static SET_RGN_ERR: AtomicU64 = AtomicU64::new(0);
/// When true, HWND region is cleared so magnify / tooltip / pending menu
/// can paint outside the CSS pill. Idle dock keeps a pill-shaped clip as
/// shape insurance over the transparent WebView2 surface.
static REGION_RELAXED: AtomicBool = AtomicBool::new(false);
/// Held from context-menu pre-open until overlay teardown so the click-through
/// poller cannot re-apply a pill clip while the menu is mounted but
/// `menu_overlay` state has not landed yet (or cursor left the rest hit-box).
static MENU_REGION_HOLD: AtomicBool = AtomicBool::new(false);
/// WebView2 DefaultBackgroundColor already forced to alpha=0 — avoid
/// re-setting it on every hover `clear_window_rgn` (expensive / flicker).
static TRANSPARENT_BG_APPLIED: AtomicBool = AtomicBool::new(false);

fn last_win32_error() -> u32 {
    unsafe { GetLastError().0 }
}

fn store_pill_client_rect(x: f64, y: f64, width: f64, height: f64) {
    if let Ok(mut guard) = LAST_PILL_CLIENT.lock() {
        *guard = Some(PillClientRect {
            x,
            y,
            width,
            height,
        });
    }
}

fn last_pill_client_rect() -> Option<PillClientRect> {
    LAST_PILL_CLIENT
        .lock()
        .ok()
        .and_then(|guard| *guard)
}

fn read_gwl_styles(window: &WebviewWindow) -> Option<(u32, u32)> {
    let hwnd = window.hwnd().ok()?;
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 };
    let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
    Some((style, exstyle))
}

fn chrome_delta_px(window: &WebviewWindow) -> Option<(i32, i32)> {
    let inner = window.inner_size().ok()?;
    let outer = window.outer_size().ok()?;
    Some((
        outer.width as i32 - inner.width as i32,
        outer.height as i32 - inner.height as i32,
    ))
}

fn style_flag_bits(style: u32) -> String {
    format!(
        "CAPTION={} SYSMENU={} POPUP={} VISIBLE={} CLIPSIBLINGS={} CLIPCHILDREN={}",
        u8::from((style & WS_CAPTION.0) == WS_CAPTION.0),
        u8::from((style & WS_SYSMENU.0) != 0),
        u8::from((style & WS_POPUP.0) != 0),
        u8::from((style & WS_VISIBLE.0) != 0),
        u8::from((style & WS_CLIPSIBLINGS.0) != 0),
        u8::from((style & WS_CLIPCHILDREN.0) != 0),
    )
}

fn log_chrome_state(phase: &str, window: &WebviewWindow) {
    let styles = read_gwl_styles(window);
    let delta = chrome_delta_px(window);
    match (styles, delta) {
        (Some((style, exstyle)), Some((dw, dh))) => {
            log::info!(
                "[win-backdrop] chrome {phase}: STYLE=0x{style:08x} EXSTYLE=0x{exstyle:08x} \
                 {} chrome_delta=({dw},{dh})",
                style_flag_bits(style),
            );
        }
        (Some((style, exstyle)), None) => {
            log::info!(
                "[win-backdrop] chrome {phase}: STYLE=0x{style:08x} EXSTYLE=0x{exstyle:08x} \
                 {} chrome_delta=?",
                style_flag_bits(style),
            );
        }
        _ => log::warn!("[win-backdrop] chrome {phase}: unable to read HWND styles"),
    }
}

/// Extra context when DOM reports an implausible pill (e.g. 40×91 capsule).
pub(crate) fn log_implausible_pill_chrome(window: &WebviewWindow, width: f64, height: f64) {
    let stored = {
        let state = window.state::<AppsState>();
        let w = *state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let h = *state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        (w, h)
    };
    let inner = window
        .inner_size()
        .ok()
        .map(|s| format!("{}x{}", s.width, s.height))
        .unwrap_or_else(|| "?".into());
    let outer = window
        .outer_size()
        .ok()
        .map(|s| format!("{}x{}", s.width, s.height))
        .unwrap_or_else(|| "?".into());
    let style = read_gwl_styles(window)
        .map(|(s, _)| format!("0x{s:08x}"))
        .unwrap_or_else(|| "?".into());
    let delta = chrome_delta_px(window)
        .map(|(dw, dh)| format!("({dw},{dh})"))
        .unwrap_or_else(|| "?".into());
    log::warn!(
        "[win-backdrop] implausible pill context: reported={width:.1}x{height:.1} \
         stored_pill={:.1}x{:.1} inner={inner} outer={outer} GWL_STYLE={style} chrome_delta={delta}",
        stored.0,
        stored.1,
    );
}

/// Rewrite overlapped caption chrome to a frameless `WS_POPUP` shell.
///
/// Do **not** strip down to `WS_CLIPSIBLINGS` alone (`0x04000000`) — that
/// collapses the client and the next DOM measure reports a ~40×91 capsule.
///
/// Returns `Ok(true)` when style bits were rewritten, `Ok(false)` when already
/// a caption-free popup.
fn rewrite_frameless_popup_style(window: &WebviewWindow) -> Result<bool, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 };
    let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0 || (style & WS_SYSMENU.0) != 0;
    let is_popup = (style & WS_POPUP.0) != 0;
    if !has_caption && is_popup {
        log::info!(
            "[win-backdrop] chrome rewrite skipped (already POPUP, no caption): STYLE=0x{style:08x}"
        );
        return Ok(false);
    }

    let desired = WS_POPUP.0 | WS_CLIPSIBLINGS.0 | WS_CLIPCHILDREN.0 | (style & WS_VISIBLE.0);
    let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, desired as isize) };
    if previous == 0 && style != 0 {
        let gle = last_win32_error();
        if gle != 0 {
            return Err(format!(
                "SetWindowLongPtrW(GWL_STYLE) failed gle={gle} was=0x{style:08x} desired=0x{desired:08x}"
            ));
        }
    }
    if let Err(err) = unsafe {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    } {
        log::warn!("[win-backdrop] SetWindowPos(FRAMECHANGED) after style rewrite: {err}");
    }

    let after = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 };
    let after_caption =
        (after & WS_CAPTION.0) == WS_CAPTION.0 || (after & WS_SYSMENU.0) != 0;
    let after_popup = (after & WS_POPUP.0) != 0;
    log::info!(
        "[win-backdrop] chrome rewritten → STYLE=0x{after:08x} {} (was 0x{style:08x})",
        style_flag_bits(after),
    );
    if after_caption || !after_popup {
        return Err(format!(
            "frameless popup rewrite did not stick: STYLE=0x{after:08x} {} desired=0x{desired:08x}",
            style_flag_bits(after),
        ));
    }
    Ok(true)
}

/// Re-apply intended inner size/position after NC chrome changes so client
/// metrics match the dock formula (NOSIZE+FRAMECHANGED can inflate inner).
fn reapply_dock_frame_after_chrome(
    window: &WebviewWindow,
    window_width: f64,
    window_height: f64,
    position: DockPosition,
) -> Result<(), String> {
    let resized = apply_dock_window_frame(window, window_width, window_height, position)?;
    if let Some((dw, dh)) = chrome_delta_px(window) {
        log::info!(
            "[win-backdrop] frame reapplied after chrome: target={window_width:.1}x{window_height:.1} \
             resized={resized} chrome_delta=({dw},{dh})"
        );
    } else {
        log::info!(
            "[win-backdrop] frame reapplied after chrome: target={window_width:.1}x{window_height:.1} \
             resized={resized}"
        );
    }
    Ok(())
}

/// Support snapshot for diagnostics clipboard / log correlation.
pub fn windows_backdrop_snapshot(window: &WebviewWindow) -> WindowsBackdropSnapshot {
    let scale = window.scale_factor().ok();
    let inner = window.inner_size().ok().map(|s| (s.width, s.height));
    let outer = window.outer_size().ok().map(|s| (s.width, s.height));
    let outer_pos = window.outer_position().ok().map(|p| (p.x, p.y));
    let styles = read_gwl_styles(window);
    let stored_pill = {
        let state = window.state::<AppsState>();
        let w = *state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let h = *state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if w >= 1.0 && h >= 1.0 {
            Some((w, h))
        } else {
            None
        }
    };
    let last_pill = last_pill_client_rect().map(|r| WindowsPillRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    });
    WindowsBackdropSnapshot {
        last_pill_client: last_pill,
        menu_overlay_active: menu_overlay_active(window),
        region_relaxed: REGION_RELAXED.load(Ordering::Relaxed),
        menu_region_hold: MENU_REGION_HOLD.load(Ordering::Relaxed),
        scale_factor: scale,
        inner_size_px: inner,
        outer_size_px: outer,
        outer_position_px: outer_pos,
        stored_pill_dip: stored_pill,
        dock_position: format!("{:?}", current_dock_position(window)),
        sync_vibrancy_calls: SYNC_VIBRANCY_CALLS.load(Ordering::Relaxed),
        set_rgn_ok_count: SET_RGN_OK.load(Ordering::Relaxed),
        set_rgn_err_count: SET_RGN_ERR.load(Ordering::Relaxed),
        gwl_style: styles.map(|(s, _)| s),
        gwl_exstyle: styles.map(|(_, e)| e),
        chrome_delta_px: chrome_delta_px(window),
        mica_enabled: false,
    }
}

fn menu_blocks_pill_clip(window: &WebviewWindow) -> bool {
    MENU_REGION_HOLD.load(Ordering::SeqCst) || menu_overlay_active(window)
}

/// Clears or restores the pill-shaped `SetWindowRgn` clip.
///
/// `relaxed = true` (hover / pre-menu): full HWND so magnify, tooltips, and
/// context menus are not clipped. `relaxed = false` (idle): clip to the CSS
/// pill (shape insurance; tint is CSS, not Mica).
///
/// `menu_hold = Some(true)` before mounting a context menu; `Some(false)` when
/// the menu fully closes. While hold/overlay is active, requests to tighten
/// the clip (poller leave, leaveDock) are deferred so the menu is never cut.
pub fn set_dock_region_relaxed(
    window: &WebviewWindow,
    relaxed: bool,
    menu_hold: Option<bool>,
) -> Result<(), String> {
    if let Some(hold) = menu_hold {
        let prev_hold = MENU_REGION_HOLD.swap(hold, Ordering::SeqCst);
        if prev_hold != hold {
            log::info!("[win-backdrop] menu_region_hold {prev_hold} → {hold}");
        }
    }

    if !relaxed && menu_blocks_pill_clip(window) {
        // Remember "not hovered" for after the menu closes, but keep HWND clear.
        REGION_RELAXED.store(false, Ordering::SeqCst);
        set_expand_for_hover(true);
        log::info!(
            "[win-backdrop] region tighten deferred (menu_hold={} overlay={})",
            MENU_REGION_HOLD.load(Ordering::Relaxed),
            menu_overlay_active(window)
        );
        return clear_window_rgn(window);
    }

    let prev = REGION_RELAXED.swap(relaxed, Ordering::SeqCst);
    set_expand_for_hover(relaxed || menu_blocks_pill_clip(window));

    if prev == relaxed {
        // Still re-apply: a concurrent sync may have restored the pill clip
        // while the flag was already true (menu race / geometry sync).
        if relaxed || menu_blocks_pill_clip(window) {
            return clear_window_rgn(window);
        }
        return sync_pill_window_rgn(window);
    }
    log::info!("[win-backdrop] region_relaxed {prev} → {relaxed}");

    // Rest HWND == pill; grow for magnify/tooltip while hovered, shrink on leave
    // (menu overlay path uses ensure_window_fits / shrink_to_pill separately).
    if !menu_overlay_active(window) {
        let state = window.state::<AppsState>();
        let pill_width = *state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pill_height = *state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pill_width >= 1.0 && pill_height >= 1.0 {
            let icon_size_dip = current_icon_size_dip(window);
            match resize_dock_window_for_pill(window, pill_width, pill_height, icon_size_dip) {
                Ok(changed) => {
                    if changed {
                        log::info!(
                            "[win-backdrop] hover_frame resized={} expand={}",
                            changed,
                            relaxed
                        );
                    }
                }
                Err(err) => log::warn!("[win-backdrop] hover_frame resize failed: {err}"),
            }
        }
    }

    if relaxed || menu_blocks_pill_clip(window) {
        clear_window_rgn(window)
    } else {
        sync_pill_window_rgn(window)
    }
}

/// Clears `MENU_REGION_HOLD` without touching hover `REGION_RELAXED`, then
/// re-syncs the clip (pill if idle, clear if still hovered).
pub fn clear_dock_menu_region_hold(window: &WebviewWindow) -> Result<(), String> {
    let prev = MENU_REGION_HOLD.swap(false, Ordering::SeqCst);
    if prev {
        log::info!("[win-backdrop] menu_region_hold true → false (overlay closed)");
    }
    set_expand_for_hover(REGION_RELAXED.load(Ordering::SeqCst) || menu_overlay_active(window));
    sync_pill_window_rgn(window)
}

/// Re-applies region clip for the current mode after any native resize.
/// Idle → pill region (no Mica); hover/menu → full HWND.
pub fn refresh_windows_backdrop(window: &WebviewWindow) -> Result<(), String> {
    log::info!("[win-backdrop] refresh: sync region for current mode (mica off)");
    sync_pill_window_rgn(window)
}

fn set_dwm_corner_preference(
    window: &WebviewWindow,
    preference: DWM_WINDOW_CORNER_PREFERENCE,
) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    // Only DONOTROUND is used — DWMWCP_ROUND on a full HWND paints a dark
    // rounded shell in hover/menu margins.
    let label = if preference == DWMWCP_DONOTROUND {
        "DONOTROUND"
    } else {
        "OTHER"
    };
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const DWM_WINDOW_CORNER_PREFERENCE as *const _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
        .map_err(|e| {
            let gle = last_win32_error();
            log::error!("[win-backdrop] DwmSetWindowAttribute({label}) failed: {e} gle={gle}");
            e.to_string()
        })?;
    }
    log::debug!("[win-backdrop] DWM corner preference={label}");
    Ok(())
}

fn assert_transparent_webview_bg(window: &WebviewWindow, force: bool) {
    if !force && TRANSPARENT_BG_APPLIED.load(Ordering::Relaxed) {
        return;
    }
    if let Err(err) = window.set_background_color(Some(Color(0, 0, 0, 0))) {
        log::warn!("[win-backdrop] set_background_color(transparent) failed: {err}");
        return;
    }
    TRANSPARENT_BG_APPLIED.store(true, Ordering::Relaxed);
}

/// Space title — empty `""` lets WebView2 fall back to HTML `<title>` for the
/// ghost titlebar text on buggy runtimes.
fn clear_dock_window_title(window: &WebviewWindow) {
    if let Err(err) = window.set_title(" ") {
        log::warn!("[win-backdrop] set_title(\" \") failed: {err}");
    }
}

/// Frameless dock chrome: Tauri decorations off, then rewrite Win32 style to
/// `WS_POPUP|CLIPSIBLINGS|CLIPCHILDREN` when caption bits remain (Tao often
/// leaves `0x04CB0000` after `set_decorations(false)`).
///
/// Always re-applies `window_width`×`window_height` afterward — removing NC
/// chrome with `SWP_NOSIZE` can inflate the client past the dock formula.
fn ensure_frameless_dock_chrome(
    window: &WebviewWindow,
    window_width: f64,
    window_height: f64,
    position: DockPosition,
) -> Result<(), String> {
    log_chrome_state("pre", window);

    if let Err(err) = window.set_decorations(false) {
        log::warn!("[win-backdrop] set_decorations(false) failed: {err}");
    }
    if let Err(err) = window.set_shadow(false) {
        log::warn!("[win-backdrop] set_shadow(false) failed: {err}");
    }
    clear_dock_window_title(window);
    assert_transparent_webview_bg(window, true);
    // Drop any system backdrop before first paint — Mica leaves dark corners
    // outside the CSS/RGB pill even when SetWindowRgn is applied.
    if let Err(err) = clear_dock_mica(window) {
        log::warn!("[win-backdrop] clear_mica during chrome setup: {err}");
    }

    log_chrome_state("post set_decorations", window);
    let rewritten = rewrite_frameless_popup_style(window)?;
    // Re-frame whether or not we rewrote — set_decorations alone can change NC.
    reapply_dock_frame_after_chrome(window, window_width, window_height, position)?;
    log_chrome_state(
        if rewritten {
            "post rewrite+frame"
        } else {
            "post frame (no rewrite)"
        },
        window,
    );

    match tauri::webview_version() {
        Ok(v) => {
            log::info!("[win-backdrop] webview2_version={v}");
            if let Some(major) = v
                .split('.')
                .next()
                .and_then(|s| s.parse::<u32>().ok())
            {
                if major > 0 && major < 146 {
                    log::warn!(
                        "[win-backdrop] WebView2 {v} may show a ghost titlebar on transparent \
                         frameless windows — update the Evergreen WebView2 Runtime"
                    );
                }
            }
        }
        Err(e) => log::warn!("[win-backdrop] webview2_version unavailable: {e}"),
    }
    Ok(())
}

/// Re-assert frameless popup after first map — styles can drift on show.
/// If style changes, re-size the frame and refresh the pill region.
fn reassert_dock_chrome_after_show(
    window: &WebviewWindow,
    window_width: f64,
    window_height: f64,
    position: DockPosition,
) {
    clear_dock_window_title(window);
    assert_transparent_webview_bg(window, true);
    match rewrite_frameless_popup_style(window) {
        Ok(true) => {
            if let Err(err) =
                reapply_dock_frame_after_chrome(window, window_width, window_height, position)
            {
                log::warn!("[win-backdrop] frame reapply after show failed: {err}");
            }
            if let Err(err) = refresh_windows_backdrop(window) {
                log::warn!("[win-backdrop] region refresh after show chrome failed: {err}");
            }
        }
        Ok(false) => {}
        Err(err) => log::warn!("[win-backdrop] chrome rewrite after show failed: {err}"),
    }
    log_chrome_state("after show", window);
}

/// Ensure system backdrop is off. Win11 Mica paints the full HWND and does
/// not clip under a custom 28px CSS/RGB pill — dark corners outside the ring.
fn clear_dock_mica(window: &WebviewWindow) -> Result<(), String> {
    use window_vibrancy::clear_mica;

    match clear_mica(window) {
        Ok(()) => {
            log::info!("[win-backdrop] clear_mica ok (mica disabled)");
            Ok(())
        }
        Err(e) => {
            log::error!("[win-backdrop] clear_mica failed: {e}");
            Err(e.to_string())
        }
    }
}

fn menu_overlay_active(window: &WebviewWindow) -> bool {
    let state = window.state::<AppsState>();
    let guard = state
        .menu_overlay
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.is_active()
}

/// Clears the custom region so the full HWND (incl. menu overlay) is visible.
/// Also re-asserts no Mica so expanded margins stay transparent.
fn clear_window_rgn(window: &WebviewWindow) -> Result<(), String> {
    if let Err(err) = clear_dock_mica(window) {
        log::warn!("[win-backdrop] clear_mica before region clear: {err}");
    }
    // Transparent bg is set at setup; only re-apply once if somehow unset.
    assert_transparent_webview_bg(window, false);
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    // None removes the region; system does not take ownership.
    let ok = unsafe { SetWindowRgn(hwnd, None, true) };
    if ok == 0 {
        let gle = last_win32_error();
        log::error!("[win-backdrop] SetWindowRgn(clear) failed gle={gle}");
        return Err(format!("SetWindowRgn(clear) failed gle={gle}"));
    }
    // Never DWMWCP_ROUND here — it paints a dark rounded HWND shell.
    set_dwm_corner_preference(window, DWMWCP_DONOTROUND)?;
    log::info!(
        "[win-backdrop] region cleared + mica off + DONOTROUND (hover/menu margins transparent)"
    );
    Ok(())
}

/// Fallback pill origin in client DIP when DOM sync has not run yet.
fn fallback_pill_client_rect(
    window: &WebviewWindow,
    width: f64,
    height: f64,
) -> Result<PillClientRect, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let client_w = inner.width as f64 / scale;
    let client_h = inner.height as f64 / scale;
    let inset = DOCK_EDGE_INSET_DIP;
    let position = current_dock_position(window);
    let (x, y) = match position {
        DockPosition::Bottom => ((client_w - width) * 0.5, client_h - inset - height),
        DockPosition::Top => ((client_w - width) * 0.5, inset),
        DockPosition::Left => (inset, (client_h - height) * 0.5),
        DockPosition::Right => (client_w - inset - width, (client_h - height) * 0.5),
    };
    Ok(PillClientRect {
        x,
        y,
        width,
        height,
    })
}

fn resolve_pill_client_rect(window: &WebviewWindow) -> Result<PillClientRect, String> {
    if let Some(rect) = last_pill_client_rect() {
        if rect.width >= 1.0 && rect.height >= 1.0 {
            return Ok(rect);
        }
    }
    let state = window.state::<AppsState>();
    let width = *state
        .pill_width_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let height = *state
        .pill_height_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if width < 1.0 || height < 1.0 {
        return Err("pill dims not ready for window region".to_string());
    }
    fallback_pill_client_rect(window, width, height)
}

/// Clips HWND to the rounded CSS pill. Cleared while a menu overlay is open,
/// menu-hold is set, or the region is relaxed (hover / magnify / tooltip).
fn sync_pill_window_rgn(window: &WebviewWindow) -> Result<(), String> {
    if menu_blocks_pill_clip(window) || REGION_RELAXED.load(Ordering::SeqCst) {
        log::info!(
            "[win-backdrop] sync_rgn: clear (menu={} hold={} relaxed={})",
            menu_overlay_active(window),
            MENU_REGION_HOLD.load(Ordering::Relaxed),
            REGION_RELAXED.load(Ordering::Relaxed)
        );
        return clear_window_rgn(window);
    }

    let rect = match resolve_pill_client_rect(window) {
        Ok(rect) => rect,
        Err(err) => {
            // Startup race before store_pill_dims — leave unclipped briefly.
            log::warn!("[win-backdrop] sync_rgn skipped (no pill yet): {err}");
            return Ok(());
        }
    };

    set_window_rgn_to_pill(window, rect.x, rect.y, rect.width, rect.height)
}

fn set_window_rgn_to_pill(
    window: &WebviewWindow,
    x_dip: f64,
    y_dip: f64,
    width_dip: f64,
    height_dip: f64,
) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let left = (x_dip * scale).round() as i32;
    let top = (y_dip * scale).round() as i32;
    // CreateRoundRectRgn right/bottom are exclusive.
    let right = ((x_dip + width_dip) * scale).round() as i32;
    let bottom = ((y_dip + height_dip) * scale).round() as i32;
    let diameter = ((PILL_CORNER_RADIUS_DIP * 2.0) * scale).round().max(2.0) as i32;

    if right <= left || bottom <= top {
        log::warn!(
            "[win-backdrop] SetWindowRgn skipped empty rect dip=({x_dip:.1},{y_dip:.1} \
             {width_dip:.1}x{height_dip:.1}) px=({left},{top})-({right},{bottom}) scale={scale:.2}"
        );
        return Ok(());
    }

    // Own rounding via region — disable DWM's ~8px OS round so it doesn't
    // paint a secondary dark shell outside the 28px CSS pill.
    set_dwm_corner_preference(window, DWMWCP_DONOTROUND)?;
    // Never Mica: DWM backdrop ignores GDI region and fills dark corners
    // outside the RGB ring. Tint is CSS over transparent WebView2.
    if let Err(err) = clear_dock_mica(window) {
        log::warn!("[win-backdrop] clear_mica before pill region: {err}");
    }

    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let hrgn = unsafe { CreateRoundRectRgn(left, top, right, bottom, diameter, diameter) };
    if hrgn.is_invalid() {
        let gle = last_win32_error();
        SET_RGN_ERR.fetch_add(1, Ordering::Relaxed);
        log::error!("[win-backdrop] CreateRoundRectRgn failed gle={gle}");
        return Err(format!("CreateRoundRectRgn failed gle={gle}"));
    }
    // On success SetWindowRgn takes ownership of hrgn — do not DeleteObject.
    let ok = unsafe { SetWindowRgn(hwnd, Some(hrgn), true) };
    if ok == 0 {
        let gle = last_win32_error();
        let _ = unsafe { DeleteObject(HGDIOBJ(hrgn.0)) };
        SET_RGN_ERR.fetch_add(1, Ordering::Relaxed);
        log::error!(
            "[win-backdrop] SetWindowRgn(pill) failed gle={gle} \
             dip=({x_dip:.1},{y_dip:.1} {width_dip:.1}x{height_dip:.1}) \
             px=({left},{top})-({right},{bottom}) diameter={diameter} scale={scale:.2}"
        );
        return Err(format!("SetWindowRgn(pill) failed gle={gle}"));
    }
    SET_RGN_OK.fetch_add(1, Ordering::Relaxed);
    let style = read_gwl_styles(window)
        .map(|(s, _)| format!("0x{s:08x}"))
        .unwrap_or_else(|| "?".into());
    let delta = chrome_delta_px(window)
        .map(|(dw, dh)| format!("({dw},{dh})"))
        .unwrap_or_else(|| "?".into());
    log::info!(
        "[win-backdrop] SetWindowRgn ok dip=({x_dip:.1},{y_dip:.1} {width_dip:.1}x{height_dip:.1}) \
         px=({left},{top})-({right},{bottom}) diameter={diameter} scale={scale:.2} \
         STYLE={style} chrome_delta={delta} ok#={}",
        SET_RGN_OK.load(Ordering::Relaxed)
    );
    Ok(())
}

pub fn apply_dock_window_layer(
    window: &WebviewWindow,
    layer: DockWindowLayer,
) -> Result<(), String> {
    let on_top = matches!(layer, DockWindowLayer::AboveWindows);
    log::info!("[win-backdrop] set_always_on_top={on_top} layer={layer:?}");
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

    let scale = window.scale_factor().unwrap_or(1.0);
    log::info!(
        "[win-backdrop] setup_dock_window: position={position:?} icon_size={icon_size_dip} \
         apps={entry_count} pill={pill_width:.1}x{pill_height:.1} \
         window={window_width:.1}x{window_height:.1} scale={scale:.2} \
         rest=inset_plus_pill_no_glow"
    );

    apply_dock_window_frame(&window, window_width, window_height, position)?;
    if let (Ok(inner), Ok(outer), Ok(pos)) = (
        window.inner_size(),
        window.outer_size(),
        window.outer_position(),
    ) {
        log::info!(
            "[win-backdrop] after frame: inner={}x{} outer={}x{} pos=({},{})",
            inner.width,
            inner.height,
            outer.width,
            outer.height,
            pos.x,
            pos.y
        );
    }

    // Frameless + transparent before region/show — strips residual caption bits
    // and logs WebView2 version for ghost-titlebar triage.
    ensure_frameless_dock_chrome(&window, window_width, window_height, position)?;

    let layer = {
        let state = app.state::<crate::commands::settings::SettingsState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.dock_window_layer
    };
    apply_dock_window_layer(&window, layer)?;

    store_pill_dims(&window, pill_width, pill_height);
    if let Ok(rect) = fallback_pill_client_rect(&window, pill_width, pill_height) {
        store_pill_client_rect(rect.x, rect.y, rect.width, rect.height);
        log::info!(
            "[win-backdrop] fallback pill client=({:.1},{:.1} {:.1}x{:.1})",
            rect.x,
            rect.y,
            rect.width,
            rect.height
        );
    }

    refresh_windows_backdrop(&window)?;

    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;

    start_dock_input(window.clone());

    window.show().map_err(|e| e.to_string())?;
    // Re-assert popup chrome after show — caption bits can return.
    reassert_dock_chrome_after_show(&window, window_width, window_height, position);
    log::info!(
        "[win-backdrop] setup_dock_window: window shown snapshot={:?}",
        windows_backdrop_snapshot(&window)
    );
    Ok(())
}

pub fn ensure_window_fits_menu_overlay(
    window: &WebviewWindow,
    overlay: MenuOverlayState,
) -> Result<(), String> {
    log::info!(
        "[win-backdrop] menu_overlay side={:?} size={:.0}x{:.0} active={}",
        overlay.side,
        overlay.width_dip,
        overlay.height_dip,
        overlay.is_active()
    );
    crate::platform::geometry::ensure_window_fits_menu_overlay(window, overlay)?;
    // Menu open → clear region; menu closed → re-clip to pill.
    refresh_windows_backdrop(window)
}

pub fn shrink_dock_window_to_stored_pill(window: &WebviewWindow) -> Result<bool, String> {
    if menu_overlay_active(window) {
        log::info!("[win-backdrop] shrink_to_pill skipped: menu overlay still active");
        return Ok(false);
    }
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
        log::warn!("[win-backdrop] shrink skipped: pill dims unset");
        return Ok(false);
    }
    let icon_size_dip = current_icon_size_dip(window);
    let changed = resize_dock_window_for_pill(window, pill_width, pill_height, icon_size_dip)?;
    log::info!(
        "[win-backdrop] shrink_to_pill {pill_width:.1}x{pill_height:.1} resized={changed} \
         hover_expand={}",
        expand_for_hover()
    );
    if changed {
        refresh_windows_backdrop(window)?;
    } else {
        sync_pill_window_rgn(window)?;
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
    let call_n = SYNC_VIBRANCY_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    let icon_size_dip = current_icon_size_dip(window);
    let position = current_dock_position(window);
    if !crate::platform::geometry::pill_size_is_plausible(width, height, icon_size_dip, position)
    {
        log::warn!(
            "[win-backdrop] sync_vibrancy skipped implausible pill \
             dip=({x:.1},{y:.1} {width:.1}x{height:.1}) icon={icon_size_dip} pos={position:?}"
        );
        log_implausible_pill_chrome(window, width, height);
        return Ok(());
    }

    store_pill_dims(window, width, height);
    store_pill_client_rect(x, y, width, height);

    let changed = resize_dock_window_for_pill(window, width, height, icon_size_dip)?;
    if changed {
        // Resize can shift the pill inside the client area — re-measure is
        // the frontend's job on the next frame; still re-apply from the
        // coords we just stored (usually still correct for edge-anchored).
        refresh_windows_backdrop(window)?;
    } else {
        // Idle → pill clip (no Mica); hover/menu → clear region + full HWND.
        sync_pill_window_rgn(window)?;
    }

    // First few + every 25th + every resize — enough for support without spam.
    if call_n <= 8 || changed || call_n % 25 == 0 {
        log::info!(
            "[win-backdrop] sync_vibrancy #{call_n}: dip=({x:.1},{y:.1} {width:.1}x{height:.1}) \
             resized={changed} icon={icon_size_dip} menu={}",
            menu_overlay_active(window)
        );
    } else {
        log::debug!(
            "[win-backdrop] sync_vibrancy #{call_n}: dip=({x:.1},{y:.1} {width:.1}x{height:.1}) resized={changed}"
        );
    }
    Ok(())
}

