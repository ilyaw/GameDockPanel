//! Windows dock window setup, geometry, DWM corners, and transparent backdrop.
//!
//! ## Why this keeps changing (and what finally sticks)
//!
//! Three separate bugs were being "fixed" in a circle:
//! 1. **Ghost titlebar / pale blue bar** — Tao restores `WS_CAPTION` and/or
//!    drops `WS_EX_LAYERED`; WebView2 then paints an opaque NC strip.
//! 2. **Pale rectangular corners** outside CSS `rounded-[28px]` — WebView2
//!    fills the full HWND; CSS radius does not punch alpha. DirectComposition
//!    often ignores GDI regions on the *child* alone, so the clip must land
//!    on the top-level HWND too.
//! 3. **Rainbow / jagged RGB ring** — a hard GDI edge can alias the CSS
//!    border AA. Prefer that over opaque corner blobs; keep diameter matched
//!    to `PILL_CORNER_RADIUS_DIP` (no +2 squaring bias). Frontend insets
//!    `.dock-border-clip` by 3px on Windows (`dock-root--windows`) and skips
//!    `filter: drop-shadow` so stroke AA sits inside the hard clip instead of
//!    getting stair-stepped into a "low HD" look.
//!
//! Durable approach:
//! - Subclass: `WM_NCCALCSIZE`/`WM_NCPAINT` kill NC chrome; style changes
//!   re-assert popup + `WS_EX_LAYERED` with `SWP_FRAMECHANGED`, then force
//!   transparent WebView2 bg + invalidate (LAYERED alone can leave a white fill).
//! - Never Mica (`DONOTROUND` only).
//! - `SetWindowRgn`: RoundRect-only when HWND == pill (rest); at hover/menu
//!   `RoundRect(pill) OR (client DIFF pill AABB)` on both top-level and
//!   WebView2 child. Never fully clear the region — that re-opens pale corners.
//! - Diag poller self-heals paperclip (`OUTER_TOO_NARROW` / axis flip) by
//!   re-applying the formula rest frame once per unhealthy streak.

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use tauri::utils::config::Color;
use tauri::{App, Emitter, Manager, WebviewWindow};
use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, InvalidateRect, RedrawWindow,
    SetWindowRgn, HGDIOBJ, HRGN, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE,
    RDW_UPDATENOW, RGN_DIFF, RGN_ERROR, RGN_OR,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindow, GetWindowLongPtrW, PostMessageW, SetWindowLongPtrW, SetWindowPos,
    GW_CHILD, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, WM_APP, WM_NCCALCSIZE, WM_NCPAINT, WM_STYLECHANGED, WS_CAPTION, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TRANSPARENT, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};

use crate::commands::apps::{AppsState, MenuOverlayState};
use crate::commands::settings::{DockPosition, DockWindowLayer};
use crate::platform::geometry::{
    apply_dock_window_frame, axis_css_dims, current_dock_position, current_icon_size_dip,
    expand_for_hover, formula_window_frame_rest, resize_dock_window_for_pill, set_expand_for_hover,
    store_pill_dims, window_length_rest_dip, window_thickness_rest_dip, PILL_CORNER_RADIUS_DIP,
};

use super::input::start_dock_input;

/// After subclass restores `WS_EX_LAYERED` without a `WebviewWindow` handle,
/// the next refresh path re-asserts DefaultBackgroundColor + invalidates.
static NEED_TRANSPARENT_BG_REASSERT: AtomicBool = AtomicBool::new(false);
/// Avoid hammering formula re-apply every 2s while a paperclip condition persists.
static PAPERCLIP_SELF_HEAL_ATTEMPTED: AtomicBool = AtomicBool::new(false);

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
    /// From JS `window.devicePixelRatio` (via `report_webview_render_metrics`).
    pub frontend_device_pixel_ratio: Option<f64>,
    /// From JS `window.innerWidth` × `innerHeight` (CSS px).
    pub frontend_viewport_css: Option<(f64, f64)>,
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
    /// Decoded chrome flags for HUD / support paste.
    pub has_caption: bool,
    pub is_popup: bool,
    pub is_layered: bool,
    pub is_transparent_ex: bool,
    pub chrome_subclass_installed: bool,
    pub webview_child_class: Option<String>,
    pub chrome_repair_count: u64,
    pub layered_restore_count: u64,
    pub caption_creep_count: u64,
    /// Empty when healthy; otherwise human-readable chrome issue tags.
    /// Advisory DPI signals live in `dpi_mismatch` — never here (would flood
    /// the unhealthy poller path and paint a false chrome-failure HUD).
    pub health_issues: Vec<String>,
    pub healthy: bool,
    /// Advisory only: `|tauri_scale − devicePixelRatio| > 0.08`. Does **not**
    /// affect `healthy` / chrome repair.
    pub dpi_mismatch: Option<String>,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsPillRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug)]
struct FrontendRenderMetrics {
    device_pixel_ratio: f64,
    viewport_css_w: f64,
    viewport_css_h: f64,
}

/// Store WebView JS metrics so `[win-diag]` can flag DPI mismatches.
/// Logs only when values actually change (hover HWND resize must not spam).
pub fn store_frontend_render_metrics(
    device_pixel_ratio: f64,
    viewport_css_w: f64,
    viewport_css_h: f64,
) {
    if !device_pixel_ratio.is_finite()
        || device_pixel_ratio <= 0.0
        || !viewport_css_w.is_finite()
        || !viewport_css_h.is_finite()
        || viewport_css_w < 0.0
        || viewport_css_h < 0.0
    {
        log::warn!(
            "[win-diag] ignoring invalid frontend render metrics \
             dpr={device_pixel_ratio} viewport={viewport_css_w}x{viewport_css_h}"
        );
        return;
    }
    let metrics = FrontendRenderMetrics {
        device_pixel_ratio,
        viewport_css_w,
        viewport_css_h,
    };
    let changed = {
        let Ok(mut guard) = FRONTEND_RENDER.lock() else {
            return;
        };
        let changed = match *guard {
            Some(prev) => {
                (prev.device_pixel_ratio - metrics.device_pixel_ratio).abs() > 0.01
                    || (prev.viewport_css_w - metrics.viewport_css_w).abs() > 0.5
                    || (prev.viewport_css_h - metrics.viewport_css_h).abs() > 0.5
            }
            None => true,
        };
        if changed {
            *guard = Some(metrics);
        }
        changed
    };
    if changed {
        log::info!(
            "[win-diag] frontend render dpr={device_pixel_ratio:.3} \
             viewport_css={viewport_css_w:.1}x{viewport_css_h:.1}"
        );
    }
}

fn frontend_render_metrics() -> Option<FrontendRenderMetrics> {
    FRONTEND_RENDER
        .lock()
        .ok()
        .and_then(|guard| *guard)
}

static LAST_PILL_CLIENT: Mutex<Option<PillClientRect>> = Mutex::new(None);
/// JS render metrics for DPI blur triage (`devicePixelRatio` vs Tauri scale).
static FRONTEND_RENDER: Mutex<Option<FrontendRenderMetrics>> = Mutex::new(None);
static SYNC_VIBRANCY_CALLS: AtomicU64 = AtomicU64::new(0);
static SET_RGN_OK: AtomicU64 = AtomicU64::new(0);
static SET_RGN_ERR: AtomicU64 = AtomicU64::new(0);
static CHROME_REPAIR_COUNT: AtomicU64 = AtomicU64::new(0);
static LAYERED_RESTORE_COUNT: AtomicU64 = AtomicU64::new(0);
static CAPTION_CREEP_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_CHILD_CLASS: Mutex<Option<String>> = Mutex::new(None);
static DIAG_POLLER_STARTED: AtomicBool = AtomicBool::new(false);
/// When true, HWND region is cleared so magnify / tooltip / pending menu
/// are not clipped; also drives hover frame sizing.
static REGION_RELAXED: AtomicBool = AtomicBool::new(false);
/// Held from context-menu pre-open until overlay teardown so leaveDock /
/// poller cannot shrink the hover frame while the menu is mounted but
/// `menu_overlay` state has not landed yet (or cursor left the rest hit-box).
static MENU_REGION_HOLD: AtomicBool = AtomicBool::new(false);
/// WebView2 DefaultBackgroundColor already forced to alpha=0 — avoid
/// re-setting it on every hover region sync (expensive / flicker).
static TRANSPARENT_BG_APPLIED: AtomicBool = AtomicBool::new(false);
/// Dock HWND subclass installed (WM_NCCALCSIZE / chrome repair).
static DOCK_SUBCLASS_INSTALLED: AtomicBool = AtomicBool::new(false);
/// Guard against re-entrant style repair from WM_STYLECHANGED.
static DOCK_CHROME_REPAIRING: AtomicBool = AtomicBool::new(false);

/// Posted by the subclass when Tao restores caption / drops LAYERED.
const WM_DOCK_REPAIR_CHROME: u32 = WM_APP + 0x44D;

fn last_win32_error() -> u32 {
    unsafe { GetLastError().0 }
}

fn flush_frame_changed(hwnd: HWND) {
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    };
}

/// Force a paint after LAYERED/chrome repair so DWM does not keep an opaque
/// white strip from the previous compositing mode.
fn invalidate_dock_hwnd(hwnd: HWND) {
    let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };
    let _ = unsafe {
        RedrawWindow(
            Some(hwnd),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
        )
    };
}

fn invalidate_dock_window(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        invalidate_dock_hwnd(hwnd);
    }
}

/// Re-assert WebView2 alpha=0 + invalidate after LAYERED was restored.
fn reassert_transparent_after_layered(window: &WebviewWindow) {
    assert_transparent_webview_bg(window, true);
    invalidate_dock_window(window);
    NEED_TRANSPARENT_BG_REASSERT.store(false, Ordering::SeqCst);
}

fn consume_pending_transparent_reassert(window: &WebviewWindow) {
    if NEED_TRANSPARENT_BG_REASSERT.swap(false, Ordering::SeqCst) {
        log::info!("[win-backdrop] consuming pending transparent bg reassert after LAYERED repair");
        reassert_transparent_after_layered(window);
    }
}

/// WebView2 paints into a child HWND — that is what must be region-clipped.
/// Clipping the top-level layered window produces the rainbow AA artifact;
/// we still clip both because DirectComposition often ignores child-only
/// regions (pale corner crescents otherwise).
fn webview_clip_hwnd(toplevel: HWND) -> HWND {
    let Ok(child) = (unsafe { GetWindow(toplevel, GW_CHILD) }) else {
        if let Ok(mut guard) = LAST_CHILD_CLASS.lock() {
            *guard = None;
        }
        return toplevel;
    };
    if child.is_invalid() {
        if let Ok(mut guard) = LAST_CHILD_CLASS.lock() {
            *guard = None;
        }
        return toplevel;
    }
    let mut buf = [0u16; 96];
    let len = unsafe { GetClassNameW(child, &mut buf) };
    if len > 0 {
        let name = String::from_utf16_lossy(&buf[..len as usize]);
        // Only log on change — diag poller used to spam this every 2s.
        let mut changed = false;
        if let Ok(mut guard) = LAST_CHILD_CLASS.lock() {
            if guard.as_deref() != Some(name.as_str()) {
                *guard = Some(name.clone());
                changed = true;
            }
        }
        if changed {
            log::info!("[win-backdrop] clip target child class={name}");
        }
    }
    child
}

fn assess_chrome_health(
    style: Option<u32>,
    exstyle: Option<u32>,
    chrome_delta: Option<(i32, i32)>,
    stored_pill: Option<(f64, f64)>,
    outer: Option<(u32, u32)>,
) -> Vec<String> {
    let mut issues = Vec::new();
    if let Some(style) = style {
        let has_caption =
            (style & WS_CAPTION.0) == WS_CAPTION.0 || (style & WS_SYSMENU.0) != 0;
        let is_popup = (style & WS_POPUP.0) != 0;
        if has_caption {
            issues.push("CAPTION".into());
        }
        if !is_popup {
            issues.push("NOT_POPUP".into());
        }
    } else {
        issues.push("NO_STYLE".into());
    }
    if let Some(exstyle) = exstyle {
        if (exstyle & WS_EX_LAYERED.0) == 0 {
            issues.push("NO_LAYERED".into());
        }
    } else {
        issues.push("NO_EXSTYLE".into());
    }
    if let Some((dw, dh)) = chrome_delta {
        if dw.abs() > 1 || dh.abs() > 1 {
            issues.push(format!("CHROME_DELTA({dw},{dh})"));
        }
    }
    if let (Some((pw, ph)), Some((ow, oh))) = (stored_pill, outer) {
        // Paperclip / collapse: outer much smaller than stored pill, or
        // horizontal dock taller than wide at tiny width.
        if pw > 80.0 && (ow as f64) < pw * 0.5 {
            issues.push(format!("OUTER_TOO_NARROW({ow}<{pw:.0})"));
        }
        if ph > 40.0 && (oh as f64) < ph * 0.5 {
            issues.push(format!("OUTER_TOO_SHORT({oh}<{ph:.0})"));
        }
        if pw > ph && ow > 0 && ow < oh {
            issues.push(format!("ORIENT_FLIP? outer={ow}x{oh} pill={pw:.0}x{ph:.0}"));
        }
    }
    if !DOCK_SUBCLASS_INSTALLED.load(Ordering::Relaxed) {
        issues.push("NO_SUBCLASS".into());
    }
    issues
}

fn hwnd_needs_frameless_rewrite(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 };
    let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0 || (style & WS_SYSMENU.0) != 0;
    let is_popup = (style & WS_POPUP.0) != 0;
    has_caption || !is_popup
}

fn repair_dock_hwnd_chrome(hwnd: HWND) {
    if DOCK_CHROME_REPAIRING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _guard = scopeguard_reset_repairing();

    let mut repaired = false;
    if hwnd_needs_frameless_rewrite(hwnd) {
        CAPTION_CREEP_COUNT.fetch_add(1, Ordering::Relaxed);
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 };
        let desired = WS_POPUP.0 | WS_CLIPSIBLINGS.0 | WS_CLIPCHILDREN.0 | (style & WS_VISIBLE.0);
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, desired as isize) };
        repaired = true;
        log::warn!(
            "[win-backdrop] subclass repair STYLE 0x{style:08x} → 0x{desired:08x} \
             (caption creep #{})",
            CAPTION_CREEP_COUNT.load(Ordering::Relaxed)
        );
    }

    let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
    if (exstyle & WS_EX_LAYERED.0) == 0 {
        LAYERED_RESTORE_COUNT.fetch_add(1, Ordering::Relaxed);
        let desired = exstyle | WS_EX_LAYERED.0;
        unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired as isize) };
        repaired = true;
        // Subclass has no WebviewWindow — flag the next refresh to force
        // DefaultBackgroundColor(alpha=0) + invalidate (pale white strip).
        NEED_TRANSPARENT_BG_REASSERT.store(true, Ordering::SeqCst);
        log::warn!(
            "[win-backdrop] subclass repair LAYERED EXSTYLE 0x{exstyle:08x} → 0x{desired:08x} \
             (restore #{})",
            LAYERED_RESTORE_COUNT.load(Ordering::Relaxed)
        );
    }

    if repaired {
        CHROME_REPAIR_COUNT.fetch_add(1, Ordering::Relaxed);
        flush_frame_changed(hwnd);
        invalidate_dock_hwnd(hwnd);
    }
}

/// Tiny RAII so repair flag clears on all paths without try/finally noise.
fn scopeguard_reset_repairing() -> impl Drop {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            DOCK_CHROME_REPAIRING.store(false, Ordering::SeqCst);
        }
    }
    Reset
}

unsafe extern "system" fn dock_chrome_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uidsubclass: usize,
    _dwrefdata: usize,
) -> LRESULT {
    match msg {
        // Client area = full window rect — kills the pale/blue ghost titlebar
        // even for the brief moments Tao has restored WS_CAPTION.
        WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
        WM_NCPAINT => return LRESULT(0),
        WM_STYLECHANGED => {
            let _ = PostMessageW(Some(hwnd), WM_DOCK_REPAIR_CHROME, WPARAM(0), LPARAM(0));
        }
        m if m == WM_DOCK_REPAIR_CHROME => {
            repair_dock_hwnd_chrome(hwnd);
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn install_dock_chrome_subclass(window: &WebviewWindow) -> Result<(), String> {
    if DOCK_SUBCLASS_INSTALLED.load(Ordering::SeqCst) {
        return Ok(());
    }
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let ok = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(dock_chrome_subclass_proc),
            0x47_44_50_57, // 'GDPW'
            0,
        )
    };
    if !ok.as_bool() {
        let gle = last_win32_error();
        return Err(format!("SetWindowSubclass failed gle={gle}"));
    }
    DOCK_SUBCLASS_INSTALLED.store(true, Ordering::SeqCst);
    // Force an immediate NC recalc so the ghost bar never paints once.
    flush_frame_changed(hwnd);
    repair_dock_hwnd_chrome(hwnd);
    log::info!("[win-backdrop] chrome subclass installed (NCCALCSIZE + style repair)");
    Ok(())
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

/// Run HWND work on Tauri's UI thread. Safe from the click-through poller and
/// from setup (already-main runs the closure inline — no deadlock).
///
/// Clones `window` for the main-thread closure so callers can pass a borrow
/// without fighting the borrow checker (`&window` + `move || … &window`).
fn with_dock_main_thread<T, F>(window: &WebviewWindow, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&WebviewWindow) -> Result<T, String> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    // Separate clones: `run_on_main_thread` borrows the receiver while the
    // closure must own (and move) the handle it passes to `f`.
    let window_for_task = window.clone();
    window
        .clone()
        .run_on_main_thread(move || {
            let _ = tx.send(f(&window_for_task));
        })
        .map_err(|e| e.to_string())?;
    rx.recv()
        .map_err(|_| "dock HWND task did not complete on main thread".to_string())?
}

/// Ensure `WS_EX_LAYERED` without clobbering other exstyle bits.
///
/// Must run on the UI thread (caller marshals). Tao's
/// `set_ignore_cursor_events(false)` drops LAYERED together with
/// `WS_EX_TRANSPARENT`; without LAYERED, transparent WebView2 margins can
/// paint as an opaque white strip above the CSS pill.
///
/// After any EXSTYLE change we must `SWP_FRAMECHANGED` — otherwise DWM keeps
/// the old compositing mode and LAYERED appears set but still paints opaque.
fn ensure_layered_exstyle(window: &WebviewWindow) -> Result<bool, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
    if (exstyle & WS_EX_LAYERED.0) != 0 {
        consume_pending_transparent_reassert(window);
        return Ok(false);
    }
    let desired = exstyle | WS_EX_LAYERED.0;
    let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired as isize) };
    if previous == 0 && exstyle != 0 {
        let gle = last_win32_error();
        if gle != 0 {
            return Err(format!(
                "SetWindowLongPtrW(GWL_EXSTYLE) failed gle={gle} was=0x{exstyle:08x}"
            ));
        }
    }
    flush_frame_changed(hwnd);
    log::warn!(
        "[win-backdrop] WS_EX_LAYERED missing — restored EXSTYLE=0x{desired:08x} (was 0x{exstyle:08x}) \
         restore#={}",
        LAYERED_RESTORE_COUNT.fetch_add(1, Ordering::Relaxed) + 1
    );
    // LAYERED alone is not enough — DWM can keep an opaque white fill until
    // DefaultBackgroundColor is forced transparent and the HWND is invalidated.
    reassert_transparent_after_layered(window);
    Ok(true)
}

fn set_dock_click_through_impl(window: &WebviewWindow, ignore: bool) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
    let had_layered = (exstyle & WS_EX_LAYERED.0) != 0;
    let desired = if ignore {
        exstyle | WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0
    } else {
        (exstyle | WS_EX_LAYERED.0) & !WS_EX_TRANSPARENT.0
    };
    if desired == exstyle {
        consume_pending_transparent_reassert(window);
        return Ok(());
    }
    let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired as isize) };
    if previous == 0 && exstyle != 0 {
        let gle = last_win32_error();
        if gle != 0 {
            return Err(format!(
                "SetWindowLongPtrW(GWL_EXSTYLE) click-through failed gle={gle} \
                 was=0x{exstyle:08x} desired=0x{desired:08x}"
            ));
        }
    }
    flush_frame_changed(hwnd);
    if !had_layered {
        reassert_transparent_after_layered(window);
    } else {
        consume_pending_transparent_reassert(window);
    }
    log::debug!(
        "[win-backdrop] click-through ignore={ignore}: EXSTYLE=0x{exstyle:08x} → 0x{desired:08x}"
    );
    Ok(())
}

/// Click-through via exstyle only — never go through Tao's
/// `set_ignore_cursor_events`, which rewrites `GWL_STYLE` back to
/// overlapped+`WS_CAPTION` (white ghost titlebar) on every hover toggle.
///
/// Marshals to the UI thread (poller must not touch HWND styles directly).
pub(crate) fn set_dock_click_through(
    window: &WebviewWindow,
    ignore: bool,
) -> Result<(), String> {
    with_dock_main_thread(window, move |window| {
        set_dock_click_through_impl(window, ignore)
    })
}

fn set_dock_outer_frame_impl(
    window: &WebviewWindow,
    x: i32,
    y: i32,
    width_px: i32,
    height_px: i32,
) -> Result<(), String> {
    // Absolute floor — never lock in the ~40×91 "paperclip" client collapse.
    let width_px = width_px.max(64);
    let height_px = height_px.max(64);
    // Frameless popup: outer ≈ inner. These DIP×scale targets were historically
    // passed to Tauri `set_size` (inner); with chrome_delta≈(0,0) they match
    // the outer rect `SetWindowPos` expects.
    if let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) {
        if pos.x == x
            && pos.y == y
            && size.width as i32 == width_px
            && size.height as i32 == height_px
        {
            return Ok(());
        }
    }
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width_px,
            height_px,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .map_err(|e| format!("SetWindowPos(dock frame) failed: {e}"))?;
    Ok(())
}

/// Native outer move+size — avoids Tao `set_size`/`set_position`, which
/// restore caption chrome (`0x14CB0000`) via `WindowFlags::apply_diff`.
///
/// Marshals to the UI thread. Strips caption **before** `SetWindowPos`:
/// applying content-sized outer dims while `WS_CAPTION` is still set shrinks
/// the client into the RGB "paperclip" (~40×91).
pub(crate) fn set_dock_outer_frame(
    window: &WebviewWindow,
    x: i32,
    y: i32,
    width_px: i32,
    height_px: i32,
) -> Result<(), String> {
    with_dock_main_thread(window, move |window| {
        if style_needs_frameless_rewrite(window) {
            log::warn!(
                "[win-backdrop] caption present before SetWindowPos — rewriting POPUP first"
            );
            rewrite_frameless_popup_style_impl(window)?;
        }
        set_dock_outer_frame_impl(window, x, y, width_px, height_px)
    })
}

/// Rewrite overlapped caption chrome to a frameless `WS_POPUP` shell.
///
/// Do **not** strip down to `WS_CLIPSIBLINGS` alone (`0x04000000`) — that
/// collapses the client and the next DOM measure reports a ~40×91 capsule.
///
/// Returns `Ok(true)` when style bits were rewritten, `Ok(false)` when already
/// a caption-free popup. Always runs on the UI thread.
fn rewrite_frameless_popup_style(window: &WebviewWindow) -> Result<bool, String> {
    with_dock_main_thread(window, rewrite_frameless_popup_style_impl)
}

fn rewrite_frameless_popup_style_impl(window: &WebviewWindow) -> Result<bool, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 };
    let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0 || (style & WS_SYSMENU.0) != 0;
    let is_popup = (style & WS_POPUP.0) != 0;
    if !has_caption && is_popup {
        ensure_layered_exstyle(window)?;
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
    ensure_layered_exstyle(window)?;
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
    let delta = chrome_delta_px(window);
    // Refresh child class name for the HUD.
    if let Ok(hwnd) = window.hwnd() {
        let _ = webview_clip_hwnd(hwnd);
    }
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
    let (style, exstyle) = styles.unwrap_or((0, 0));
    let has_styles = styles.is_some();
    let has_caption = has_styles
        && ((style & WS_CAPTION.0) == WS_CAPTION.0 || (style & WS_SYSMENU.0) != 0);
    let is_popup = has_styles && (style & WS_POPUP.0) != 0;
    let is_layered = has_styles && (exstyle & WS_EX_LAYERED.0) != 0;
    let is_transparent_ex = has_styles && (exstyle & WS_EX_TRANSPARENT.0) != 0;
    let child_class = LAST_CHILD_CLASS
        .lock()
        .ok()
        .and_then(|g| g.clone());
    let health_issues = assess_chrome_health(
        styles.map(|(s, _)| s),
        styles.map(|(_, e)| e),
        delta,
        stored_pill,
        outer,
    );
    let fe = frontend_render_metrics();
    // Advisory only — must not enter `health_issues` (that path logs every 2s
    // and paints the chrome-failure HUD red wash).
    let dpi_mismatch = match (scale, fe) {
        (Some(sf), Some(m)) if (sf - m.device_pixel_ratio).abs() > 0.08 => Some(format!(
            "scale={sf:.2} dpr={:.2}",
            m.device_pixel_ratio
        )),
        _ => None,
    };
    let healthy = health_issues.is_empty();

    WindowsBackdropSnapshot {
        last_pill_client: last_pill,
        menu_overlay_active: menu_overlay_active(window),
        region_relaxed: REGION_RELAXED.load(Ordering::Relaxed),
        menu_region_hold: MENU_REGION_HOLD.load(Ordering::Relaxed),
        scale_factor: scale,
        frontend_device_pixel_ratio: fe.map(|m| m.device_pixel_ratio),
        frontend_viewport_css: fe.map(|m| (m.viewport_css_w, m.viewport_css_h)),
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
        chrome_delta_px: delta,
        mica_enabled: false,
        has_caption,
        is_popup,
        is_layered,
        is_transparent_ex,
        chrome_subclass_installed: DOCK_SUBCLASS_INSTALLED.load(Ordering::Relaxed),
        webview_child_class: child_class,
        chrome_repair_count: CHROME_REPAIR_COUNT.load(Ordering::Relaxed),
        layered_restore_count: LAYERED_RESTORE_COUNT.load(Ordering::Relaxed),
        caption_creep_count: CAPTION_CREEP_COUNT.load(Ordering::Relaxed),
        health_issues,
        healthy,
        dpi_mismatch,
    }
}

fn windows_debug_overlay_enabled(window: &WebviewWindow) -> bool {
    let state = window.state::<crate::commands::settings::SettingsState>();
    let guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.windows_debug_overlay
}

/// Force one snapshot into the log + `dock-win-diag` event (settings button).
pub fn log_windows_diag_snapshot(window: &WebviewWindow) -> WindowsBackdropSnapshot {
    let snap = windows_backdrop_snapshot(window);
    log::info!("[win-diag] MANUAL snapshot={snap:?}");
    let _ = window.emit("dock-win-diag", &snap);
    snap
}

fn start_windows_diag_poller(window: WebviewWindow) {
    if DIAG_POLLER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let mut tick: u64 = 0;
        let mut last_issues: Vec<String> = Vec::new();
        let mut last_dpi_mismatch: Option<String> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(2000));
            tick = tick.wrapping_add(1);
            // Subclass LAYERED repair only sets a flag — consume it here so
            // DefaultBackgroundColor(alpha=0) is forced within ~2s, not only
            // on the next hover/refresh.
            consume_pending_transparent_reassert(&window);
            let snap = windows_backdrop_snapshot(&window);
            let overlay = windows_debug_overlay_enabled(&window);
            let issues_changed = snap.health_issues != last_issues;
            if issues_changed {
                if snap.healthy {
                    log::info!("[win-diag] HEALTH RESTORED after {:?}", last_issues);
                    PAPERCLIP_SELF_HEAL_ATTEMPTED.store(false, Ordering::SeqCst);
                } else {
                    log::warn!(
                        "[win-diag] HEALTH BAD issues={:?} style={:?} ex={:?} delta={:?} \
                         outer={:?} pill={:?} child={:?} repairs={} layered_restores={} caption_creeps={}",
                        snap.health_issues,
                        snap.gwl_style.map(|s| format!("0x{s:08x}")),
                        snap.gwl_exstyle.map(|e| format!("0x{e:08x}")),
                        snap.chrome_delta_px,
                        snap.outer_size_px,
                        snap.stored_pill_dip,
                        snap.webview_child_class,
                        snap.chrome_repair_count,
                        snap.layered_restore_count,
                        snap.caption_creep_count,
                    );
                }
                last_issues = snap.health_issues.clone();
            }
            // Retry path: heal is rate-limited internally; on failure the flag
            // clears so the next 2s tick can try again (not only on issue edges).
            if !snap.healthy {
                self_heal_paperclip_if_needed(&window, &snap.health_issues);
            }
            let dpi_changed = snap.dpi_mismatch != last_dpi_mismatch;
            if dpi_changed {
                match &snap.dpi_mismatch {
                    Some(detail) => log::warn!("[win-diag] DPI_MISMATCH {detail}"),
                    None if last_dpi_mismatch.is_some() => {
                        log::info!("[win-diag] DPI_MISMATCH cleared");
                    }
                    None => {}
                }
                last_dpi_mismatch = snap.dpi_mismatch.clone();
            }
            // HUD needs a steady stream; unhealthy always logs periodically.
            if overlay || !snap.healthy || tick % 15 == 0 {
                if overlay || !snap.healthy {
                    log::info!(
                        "[win-diag] tick={tick} healthy={} issues={:?} dpi_mismatch={:?} pos={} \
                         outer={:?}@{:?} pill_client={:?} stored={:?} \
                         CAPTION={} POPUP={} LAYERED={} TRANSPARENT_EX={} \
                         rgn_ok={} rgn_err={} sync_vib={} relaxed={} hold={}",
                        snap.healthy,
                        snap.health_issues,
                        snap.dpi_mismatch,
                        snap.dock_position,
                        snap.outer_size_px,
                        snap.outer_position_px,
                        snap.last_pill_client,
                        snap.stored_pill_dip,
                        snap.has_caption,
                        snap.is_popup,
                        snap.is_layered,
                        snap.is_transparent_ex,
                        snap.set_rgn_ok_count,
                        snap.set_rgn_err_count,
                        snap.sync_vibrancy_calls,
                        snap.region_relaxed,
                        snap.menu_region_hold,
                    );
                } else {
                    log::info!(
                        "[win-diag] heartbeat tick={tick} healthy outer={:?} pill={:?} \
                         scale={:?} dpr_js={:?} viewport_css={:?} inner={:?} dpi_mismatch={:?}",
                        snap.outer_size_px,
                        snap.stored_pill_dip,
                        snap.scale_factor,
                        snap.frontend_device_pixel_ratio,
                        snap.frontend_viewport_css,
                        snap.inner_size_px,
                        snap.dpi_mismatch,
                    );
                }
            }
            if overlay || issues_changed || !snap.healthy || dpi_changed {
                let _ = window.emit("dock-win-diag", &snap);
            }
        }
    });
    log::info!("[win-diag] poller started (2s; emits dock-win-diag when overlay/unhealthy)");
}

/// True when health tags indicate the HWND collapsed into the classic
/// ~40×91 paperclip (or axis-flipped) geometry.
fn health_issues_look_like_paperclip(issues: &[String]) -> bool {
    issues.iter().any(|issue| {
        issue.starts_with("OUTER_TOO_NARROW")
            || issue.starts_with("OUTER_TOO_SHORT")
            || issue.starts_with("ORIENT_FLIP")
    })
}

/// Re-apply formula rest frame + pill rect + region after a detected paperclip.
/// Rate-limited to once per unhealthy streak (`PAPERCLIP_SELF_HEAL_ATTEMPTED`);
/// the flag is cleared again if the attempt fails so the next diag tick can retry.
fn self_heal_paperclip_if_needed(window: &WebviewWindow, issues: &[String]) {
    if !health_issues_look_like_paperclip(issues) {
        return;
    }
    if PAPERCLIP_SELF_HEAL_ATTEMPTED.swap(true, Ordering::SeqCst) {
        log::debug!("[win-backdrop] self-heal paperclip skipped (already attempted this streak)");
        return;
    }

    let icon_size_dip = current_icon_size_dip(window);
    let position = current_dock_position(window);
    let entries = window.state::<AppsState>().entries_snapshot();
    let (pill_width, pill_height, window_width, window_height) =
        formula_window_frame_rest(&entries, icon_size_dip, position);

    log::warn!(
        "[win-backdrop] self-heal paperclip issues={issues:?} → \
         formula pill={pill_width:.1}x{pill_height:.1} window={window_width:.1}x{window_height:.1} \
         pos={position:?} icon={icon_size_dip}"
    );

    let heal_result = (|| -> Result<(), String> {
        reassert_frameless_chrome_keep_size(window);
        apply_dock_window_frame(window, window_width, window_height, position)?;
        store_pill_dims(window, pill_width, pill_height);
        let rect = fallback_pill_client_rect(window, pill_width, pill_height)?;
        store_pill_client_rect(rect.x, rect.y, rect.width, rect.height);
        reassert_transparent_after_layered(window);
        sync_pill_window_rgn(window)?;
        Ok(())
    })();

    if let Err(err) = heal_result {
        log::warn!("[win-backdrop] self-heal paperclip failed (will retry): {err}");
        PAPERCLIP_SELF_HEAL_ATTEMPTED.store(false, Ordering::SeqCst);
    }
}

fn menu_blocks_pill_clip(window: &WebviewWindow) -> bool {
    MENU_REGION_HOLD.load(Ordering::SeqCst) || menu_overlay_active(window)
}

/// Clears or restores the pill-shaped `SetWindowRgn` clip.
///
/// `relaxed = true` (hover / pre-menu): expand HWND for magnify/tooltip, but
/// keep the combined pill∪margins region so corner crescents stay clipped.
/// `relaxed = false` (idle): shrink HWND and clip to the CSS pill.
///
/// `menu_hold = Some(true)` before mounting a context menu; `Some(false)` when
/// the menu fully closes. While hold/overlay is active, requests to shrink the
/// hover frame are deferred so the menu is never cut.
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
        // Remember "not hovered" for after the menu closes, but keep the
        // expanded frame + combined region while the menu is up.
        REGION_RELAXED.store(false, Ordering::SeqCst);
        set_expand_for_hover(true);
        log::info!(
            "[win-backdrop] region tighten deferred (menu_hold={} overlay={})",
            MENU_REGION_HOLD.load(Ordering::Relaxed),
            menu_overlay_active(window)
        );
        return sync_pill_window_rgn(window);
    }

    let prev = REGION_RELAXED.swap(relaxed, Ordering::SeqCst);
    set_expand_for_hover(relaxed || menu_blocks_pill_clip(window));

    if prev == relaxed {
        // Still re-apply: a concurrent sync may have restored a stale clip
        // while the flag was already true (menu race / geometry sync).
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

    sync_pill_window_rgn(window)
}

/// Clears `MENU_REGION_HOLD` without touching hover `REGION_RELAXED`, then
/// re-syncs the combined pill∪margins clip.
pub fn clear_dock_menu_region_hold(window: &WebviewWindow) -> Result<(), String> {
    let prev = MENU_REGION_HOLD.swap(false, Ordering::SeqCst);
    if prev {
        log::info!("[win-backdrop] menu_region_hold true → false (overlay closed)");
    }
    set_expand_for_hover(REGION_RELAXED.load(Ordering::SeqCst) || menu_overlay_active(window));
    sync_pill_window_rgn(window)
}

/// Re-applies region clip for the current mode after any native resize.
/// Always pill∪margins when expanded; RoundRect-only at rest (see
/// `create_dock_clip_hrgn`). Never a full clear — that reopens pale corners.
pub fn refresh_windows_backdrop(window: &WebviewWindow) -> Result<(), String> {
    consume_pending_transparent_reassert(window);
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
    // outside the CSS/RGB pill (and ignores GDI region clips).
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

/// Recover frameless popup after Tao stomps `GWL_STYLE` back to caption.
///
/// Re-applies the **formula** frame from the stored pill — never the current
/// `outer_size`, which may already be the collapsed paperclip client.
///
/// Even when style is already `WS_POPUP`, still re-assert `WS_EX_LAYERED` and
/// a transparent WebView2 background — Tao/`set_always_on_top` can drop
/// LAYERED without restoring caption, and that paints the empty client
/// margins as an opaque pale strip above/beside the CSS pill.
pub(crate) fn reassert_frameless_chrome_keep_size(window: &WebviewWindow) {
    clear_dock_window_title(window);
    assert_transparent_webview_bg(window, true);

    if !style_needs_frameless_rewrite(window) {
        if let Err(err) = with_dock_main_thread(window, ensure_layered_exstyle) {
            log::warn!("[win-backdrop] ensure LAYERED (style ok) failed: {err}");
        }
        return;
    }
    match rewrite_frameless_popup_style(window) {
        Ok(false) => {}
        Ok(true) => {
            let position = current_dock_position(window);
            if let Some((width_dip, height_dip)) = formula_frame_from_stored_pill(window) {
                if let Err(err) =
                    reapply_dock_frame_after_chrome(window, width_dip, height_dip, position)
                {
                    log::warn!("[win-backdrop] frame reapply after chrome failed: {err}");
                }
            } else {
                log::warn!(
                    "[win-backdrop] chrome reassert: no stored pill — skip frame reapply"
                );
            }
            // Tao may stomp style again during SetWindowPos — strip once more.
            if let Err(err) = rewrite_frameless_popup_style(window) {
                log::warn!("[win-backdrop] second chrome rewrite failed: {err}");
            }
            let accept_hits = REGION_RELAXED.load(Ordering::SeqCst)
                || MENU_REGION_HOLD.load(Ordering::SeqCst)
                || menu_overlay_active(window);
            if let Err(err) = set_dock_click_through(window, !accept_hits) {
                log::warn!("[win-backdrop] click-through after chrome reassert failed: {err}");
            }
            log_chrome_state("after chrome reassert", window);
        }
        Err(err) => log::warn!("[win-backdrop] chrome rewrite failed: {err}"),
    }
}

/// Rest-frame DIP size from the last known CSS pill (ignores hover expand).
fn formula_frame_from_stored_pill(window: &WebviewWindow) -> Option<(f64, f64)> {
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
        return None;
    }
    let position = current_dock_position(window);
    let icon_size_dip = current_icon_size_dip(window);
    let (pill_length, pill_thickness) =
        axis_css_dims(position.axis(), pill_width, pill_height);
    let window_length = window_length_rest_dip(pill_length, icon_size_dip);
    let window_thickness = window_thickness_rest_dip(pill_thickness, icon_size_dip);
    Some(axis_css_dims(
        position.axis(),
        window_length,
        window_thickness,
    ))
}

/// True when GWL_STYLE still has caption/sysmenu or is missing WS_POPUP.
fn style_needs_frameless_rewrite(window: &WebviewWindow) -> bool {
    match read_gwl_styles(window) {
        Some((style, _)) => {
            let has_caption =
                (style & WS_CAPTION.0) == WS_CAPTION.0 || (style & WS_SYSMENU.0) != 0;
            let is_popup = (style & WS_POPUP.0) != 0;
            has_caption || !is_popup
        }
        None => false,
    }
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

/// Builds the dock clip region in physical pixels.
///
/// - **Rest** (pill AABB covers the client within 2px): `RoundRect(pill)` only.
///   Avoids the failure mode where a stale/small LAST_PILL RoundRect is OR'd
///   with `client DIFF AABB` and leaves opaque pale WebView margins visible.
/// - **Expanded** (hover/menu HWND larger than pill): `RoundRect(pill) OR
///   (client DIFF pill AABB)` so magnify/tooltip/menu margins stay visible
///   while corner crescents inside the pill box stay clipped.
unsafe fn create_dock_clip_hrgn(
    pill_left: i32,
    pill_top: i32,
    pill_right: i32,
    pill_bottom: i32,
    diameter: i32,
    client_w: i32,
    client_h: i32,
) -> Result<HRGN, String> {
    unsafe fn delete_hrgn(hrgn: HRGN) {
        if !hrgn.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(hrgn.0));
        }
    }

    let pill_round =
        unsafe { CreateRoundRectRgn(pill_left, pill_top, pill_right, pill_bottom, diameter, diameter) };
    if pill_round.is_invalid() {
        return Err(format!(
            "CreateRoundRectRgn failed gle={}",
            last_win32_error()
        ));
    }

    // Resting HWND == pill: skip DIFF entirely (epsilon covers AA inflate).
    const CLIENT_COVER_EPS: i32 = 2;
    let covers_client = pill_left <= CLIENT_COVER_EPS
        && pill_top <= CLIENT_COVER_EPS
        && pill_right >= client_w - CLIENT_COVER_EPS
        && pill_bottom >= client_h - CLIENT_COVER_EPS;
    if covers_client {
        return Ok(pill_round);
    }

    let win = unsafe { CreateRectRgn(0, 0, client_w, client_h) };
    let aabb = unsafe { CreateRectRgn(pill_left, pill_top, pill_right, pill_bottom) };
    let outside = unsafe { CreateRectRgn(0, 0, 0, 0) };
    let result = unsafe { CreateRectRgn(0, 0, 0, 0) };

    if win.is_invalid() || aabb.is_invalid() || outside.is_invalid() || result.is_invalid() {
        unsafe {
            delete_hrgn(pill_round);
            delete_hrgn(win);
            delete_hrgn(aabb);
            delete_hrgn(outside);
            delete_hrgn(result);
        }
        return Err(format!(
            "CreateRectRgn failed gle={}",
            last_win32_error()
        ));
    }

    let diff = unsafe { CombineRgn(Some(outside), Some(win), Some(aabb), RGN_DIFF) };
    if diff == RGN_ERROR {
        unsafe {
            delete_hrgn(pill_round);
            delete_hrgn(win);
            delete_hrgn(aabb);
            delete_hrgn(outside);
            delete_hrgn(result);
        }
        return Err(format!("CombineRgn(DIFF) failed gle={}", last_win32_error()));
    }
    let combined = unsafe { CombineRgn(Some(result), Some(pill_round), Some(outside), RGN_OR) };
    if combined == RGN_ERROR {
        unsafe {
            delete_hrgn(pill_round);
            delete_hrgn(win);
            delete_hrgn(aabb);
            delete_hrgn(outside);
            delete_hrgn(result);
        }
        return Err(format!("CombineRgn(OR) failed gle={}", last_win32_error()));
    }

    // Temps only — `result` is owned by the caller / SetWindowRgn.
    unsafe {
        delete_hrgn(pill_round);
        delete_hrgn(win);
        delete_hrgn(aabb);
        delete_hrgn(outside);
    }
    Ok(result)
}

fn apply_hrgn_to_hwnd(hwnd: HWND, hrgn: HRGN, label: &str) -> Result<(), String> {
    let ok = unsafe { SetWindowRgn(hwnd, Some(hrgn), true) };
    if ok == 0 {
        let gle = last_win32_error();
        let _ = unsafe { DeleteObject(HGDIOBJ(hrgn.0)) };
        SET_RGN_ERR.fetch_add(1, Ordering::Relaxed);
        log::error!("[win-backdrop] SetWindowRgn({label}) failed gle={gle}");
        return Err(format!("SetWindowRgn({label}) failed gle={gle}"));
    }
    Ok(())
}

/// Applies the combined dock clip to the top-level HWND (DComp-visible) and
/// the WebView2 child. Never clears on hover/menu — see module docs.
fn set_window_rgn_to_pill(
    window: &WebviewWindow,
    x_dip: f64,
    y_dip: f64,
    width_dip: f64,
    height_dip: f64,
) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    // Inflate 1 physical px so CSS border-radius AA fringe is not chopped by
    // the hard GDI edge (SetWindowRgn has no antialiasing). Pale crescents in
    // that 1px band stay negligible; the RGB ring is inset further in CSS.
    const RGN_AA_INFLATE_PX: i32 = 1;
    let left = (x_dip * scale).round() as i32 - RGN_AA_INFLATE_PX;
    let top = (y_dip * scale).round() as i32 - RGN_AA_INFLATE_PX;
    // CreateRoundRectRgn right/bottom are exclusive.
    let right = ((x_dip + width_dip) * scale).round() as i32 + RGN_AA_INFLATE_PX;
    let bottom = ((y_dip + height_dip) * scale).round() as i32 + RGN_AA_INFLATE_PX;
    // Match CSS radius exactly — a +N squaring bias made corners stair-step more.
    // Clamp so diameter never exceeds the shorter pill axis (exclusive rect).
    let diameter = {
        let nominal = ((PILL_CORNER_RADIUS_DIP * 2.0) * scale).round().max(2.0) as i32;
        let max_fit = (right - left).min(bottom - top).max(2);
        nominal.min(max_fit)
    };

    if right <= left || bottom <= top {
        log::warn!(
            "[win-backdrop] SetWindowRgn skipped empty rect dip=({x_dip:.1},{y_dip:.1} \
             {width_dip:.1}x{height_dip:.1}) px=({left},{top})-({right},{bottom}) scale={scale:.2}"
        );
        return Ok(());
    }

    set_dwm_corner_preference(window, DWMWCP_DONOTROUND)?;
    if let Err(err) = clear_dock_mica(window) {
        log::warn!("[win-backdrop] clear_mica before pill region: {err}");
    }
    if style_needs_frameless_rewrite(window) {
        log::warn!("[win-backdrop] caption creep before SetWindowRgn — reasserting POPUP");
        reassert_frameless_chrome_keep_size(window);
    }

    with_dock_main_thread(window, move |window| {
        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        let inner = window.inner_size().map_err(|e| e.to_string())?;
        let client_w = inner.width as i32;
        let client_h = inner.height as i32;
        let clip = webview_clip_hwnd(hwnd);

        let rest = !REGION_RELAXED.load(Ordering::SeqCst) && !menu_blocks_pill_clip(window);
        const CLIENT_COVER_EPS: i32 = 2;
        let covers_client = left <= CLIENT_COVER_EPS
            && top <= CLIENT_COVER_EPS
            && right >= client_w - CLIENT_COVER_EPS
            && bottom >= client_h - CLIENT_COVER_EPS;

        // Only rewrite the clip when the pill looks like a paperclip *stub*
        // (~half the client or less on the long axis). A mere !covers during
        // hover/menu shrink races would otherwise flash a full-client RoundRect
        // over still-expanded HWND margins (pale corners).
        let pill_w = (right - left).max(0);
        let pill_h = (bottom - top).max(0);
        let stub_in_client = client_w > 2
            && client_h > 2
            && ((client_w >= client_h && pill_w * 2 < client_w)
                || (client_h > client_w && pill_h * 2 < client_h));

        let (clip_left, clip_top, clip_right, clip_bottom, clip_diameter) =
            if rest && !covers_client && stub_in_client {
                log::warn!(
                    "[win-backdrop] rest clip: stub pill inside larger client — \
                     using full-client RoundRect (avoid pale stub∪margins) \
                     pill=({left},{top})-({right},{bottom}) client={client_w}x{client_h}"
                );
                let l = -RGN_AA_INFLATE_PX;
                let t = -RGN_AA_INFLATE_PX;
                let r = client_w + RGN_AA_INFLATE_PX;
                let b = client_h + RGN_AA_INFLATE_PX;
                let nominal = ((PILL_CORNER_RADIUS_DIP * 2.0) * scale).round().max(2.0) as i32;
                let max_fit = (r - l).min(b - t).max(2);
                (l, t, r, b, nominal.min(max_fit))
            } else {
                (left, top, right, bottom, diameter)
            };

        let hrgn_parent = unsafe {
            create_dock_clip_hrgn(
                clip_left,
                clip_top,
                clip_right,
                clip_bottom,
                clip_diameter,
                client_w,
                client_h,
            )
        }?;
        // Parent first — DirectComposition respects the top-level region.
        apply_hrgn_to_hwnd(hwnd, hrgn_parent, "toplevel")?;

        if clip != hwnd {
            let hrgn_child = unsafe {
                create_dock_clip_hrgn(
                    clip_left,
                    clip_top,
                    clip_right,
                    clip_bottom,
                    clip_diameter,
                    client_w,
                    client_h,
                )
            }?;
            apply_hrgn_to_hwnd(clip, hrgn_child, "webview child")?;
        }

        SET_RGN_OK.fetch_add(1, Ordering::Relaxed);
        let style = read_gwl_styles(window)
            .map(|(s, _)| format!("0x{s:08x}"))
            .unwrap_or_else(|| "?".into());
        let delta = chrome_delta_px(window)
            .map(|(dw, dh)| format!("({dw},{dh})"))
            .unwrap_or_else(|| "?".into());
        log::info!(
            "[win-backdrop] SetWindowRgn(pill∪margins) ok dip=({x_dip:.1},{y_dip:.1} {width_dip:.1}x{height_dip:.1}) \
             px=({clip_left},{clip_top})-({clip_right},{clip_bottom}) diameter={clip_diameter} \
             client={client_w}x{client_h} scale={scale:.2} STYLE={style} chrome_delta={delta} ok#={}",
            SET_RGN_OK.load(Ordering::Relaxed)
        );
        Ok(())
    })
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
    // Near-edge inset is outside the HWND on Windows — pill sits flush to the
    // near client edge (CSS wrapper has no pt-2/pb-2/pl-2/pr-2 either).
    let position = current_dock_position(window);
    let (x, y) = match position {
        DockPosition::Bottom => ((client_w - width) * 0.5, client_h - height),
        DockPosition::Top => ((client_w - width) * 0.5, 0.0),
        DockPosition::Left => (0.0, (client_h - height) * 0.5),
        DockPosition::Right => (client_w - width, (client_h - height) * 0.5),
    };
    Ok(PillClientRect {
        x,
        y,
        width,
        height,
    })
}

fn resolve_pill_client_rect(window: &WebviewWindow) -> Result<PillClientRect, String> {
    // Prefer latest measured size, but always re-derive origin from the current
    // client + dock position. After hover expand/shrink, LAST_PILL's x/y can
    // lag one frame behind the new HWND (still centered for the *previous*
    // width) — clipping with that stale AABB leaves pale strips beside the
    // real pill and fails to mask its corner crescents.
    let (width, height) = if let Some(rect) = last_pill_client_rect() {
        if rect.width >= 1.0 && rect.height >= 1.0 {
            (rect.width, rect.height)
        } else {
            stored_pill_size(window)?
        }
    } else {
        stored_pill_size(window)?
    };
    if width < 1.0 || height < 1.0 {
        return Err("pill dims not ready for window region".to_string());
    }
    fallback_pill_client_rect(window, width, height)
}

fn stored_pill_size(window: &WebviewWindow) -> Result<(f64, f64), String> {
    let state = window.state::<AppsState>();
    let width = *state
        .pill_width_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let height = *state
        .pill_height_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok((width, height))
}

/// Re-applies the combined pill∪margins clip from the latest measured pill.
fn sync_pill_window_rgn(window: &WebviewWindow) -> Result<(), String> {
    let rect = match resolve_pill_client_rect(window) {
        Ok(rect) => rect,
        Err(err) => {
            // Startup race before store_pill_dims — leave unclipped briefly.
            log::warn!("[win-backdrop] sync_rgn skipped (no pill yet): {err}");
            return Ok(());
        }
    };

    if menu_blocks_pill_clip(window) || REGION_RELAXED.load(Ordering::SeqCst) {
        log::info!(
            "[win-backdrop] sync_rgn: pill∪margins (menu={} hold={} relaxed={})",
            menu_overlay_active(window),
            MENU_REGION_HOLD.load(Ordering::Relaxed),
            REGION_RELAXED.load(Ordering::Relaxed)
        );
    }

    set_window_rgn_to_pill(window, rect.x, rect.y, rect.width, rect.height)
}

pub fn apply_dock_window_layer(
    window: &WebviewWindow,
    layer: DockWindowLayer,
) -> Result<(), String> {
    let on_top = matches!(layer, DockWindowLayer::AboveWindows);
    log::info!("[win-backdrop] set_always_on_top={on_top} layer={layer:?}");
    window
        .set_always_on_top(on_top)
        .map_err(|e| e.to_string())?;

    // Tao `apply_diff` rewrites GWL_STYLE/EXSTYLE from WindowFlags and can
    // restore WS_CAPTION + drop TRANSPARENT|LAYERED — same ghost titlebar /
    // broken click-through as hover resize. Repair after every layer change
    // (setup and Settings → «Слой отображения»).
    reassert_frameless_chrome_keep_size(window);
    let accept_hits = REGION_RELAXED.load(Ordering::SeqCst)
        || MENU_REGION_HOLD.load(Ordering::SeqCst)
        || menu_overlay_active(window);
    set_dock_click_through(window, !accept_hits)?;
    Ok(())
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
         rest=pill_only_edge_inset_outer"
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

    let layer = {
        let state = app.state::<crate::commands::settings::SettingsState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.dock_window_layer
    };
    // always_on_top goes through Tao and may restore caption / wipe exstyle —
    // strip chrome and re-apply click-through afterward.
    apply_dock_window_layer(&window, layer)?;

    // Frameless + transparent before region/show — strips residual caption bits
    // and logs WebView2 version for ghost-titlebar triage.
    ensure_frameless_dock_chrome(&window, window_width, window_height, position)?;

    // Permanent NC kill + style repair — stops the caption/LAYERED thrash that
    // painted the pale blue bar above the RGB pill.
    if let Err(err) = install_dock_chrome_subclass(&window) {
        log::warn!("[win-backdrop] chrome subclass failed (falling back to poll repair): {err}");
    }

    // After every Tao style rewrite path — native click-through only (no
    // set_ignore_cursor_events), so TRANSPARENT|LAYERED stick.
    set_dock_click_through(&window, true)?;

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

    start_dock_input(window.clone());
    start_windows_diag_poller(window.clone());

    window.show().map_err(|e| e.to_string())?;
    // Re-assert popup chrome after show — caption bits can return.
    reassert_dock_chrome_after_show(&window, window_width, window_height, position);
    // show()/reassert may touch styles via Tao — restore click-through + pill clip.
    if let Err(err) = set_dock_click_through(&window, true) {
        log::warn!("[win-backdrop] click-through after show failed: {err}");
    }
    if let Err(err) = refresh_windows_backdrop(&window) {
        log::warn!("[win-backdrop] pill region after show failed: {err}");
    }
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
    refresh_windows_backdrop(window)?;
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
    // Always re-sync region from the measured DOM rect (idle → pill clip).
    refresh_windows_backdrop(window)?;

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

