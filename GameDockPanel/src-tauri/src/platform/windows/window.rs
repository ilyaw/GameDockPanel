//! Windows dock window setup, geometry, DWM corners, and Mica backdrop.
//!
//! Mica applies to the full HWND. To match macOS (blur only under the CSS
//! pill), we clip the window with `SetWindowRgn` to the measured rounded
//! pill — otherwise glow-bleed / magnify margins show as a dark Mica
//! "shell" outside the RGB frame. Region is cleared while hovered or a
//! context-menu overlay is open; Mica is dropped and DWM stays
//! `DONOTROUND` so expanded margins stay transparent (not a dark rounded
//! HWND shell).

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
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
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
/// can paint outside the CSS pill. Idle dock keeps a pill clip so Mica
/// does not show as a dark shell outside the RGB frame.
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

/// Support snapshot for diagnostics clipboard / log correlation.
pub fn windows_backdrop_snapshot(window: &WebviewWindow) -> WindowsBackdropSnapshot {
    let scale = window.scale_factor().ok();
    let inner = window.inner_size().ok().map(|s| (s.width, s.height));
    let outer = window.outer_size().ok().map(|s| (s.width, s.height));
    let outer_pos = window.outer_position().ok().map(|p| (p.x, p.y));
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
    }
}

fn menu_blocks_pill_clip(window: &WebviewWindow) -> bool {
    MENU_REGION_HOLD.load(Ordering::SeqCst) || menu_overlay_active(window)
}

/// Clears or restores the pill-shaped `SetWindowRgn` clip.
///
/// `relaxed = true` (hover / pre-menu): full HWND so magnify, tooltips, and
/// context menus are not clipped. `relaxed = false` (idle): clip to the CSS
/// pill so Mica does not paint a dark shell outside the RGB frame.
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

/// Re-applies backdrop for the current clip mode after any native resize.
/// Idle → Mica + pill region; hover/menu → clear Mica + full HWND.
pub fn refresh_windows_backdrop(window: &WebviewWindow) -> Result<(), String> {
    log::info!("[win-backdrop] refresh: sync region/mica for current mode");
    sync_pill_window_rgn(window)
}

fn set_dwm_corner_preference(
    window: &WebviewWindow,
    preference: DWM_WINDOW_CORNER_PREFERENCE,
) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    // Only DONOTROUND is used — DWMWCP_ROUND on a full HWND paints a dark
    // rounded shell in hover/menu margins after Mica is cleared.
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

/// Full frameless pass — call once before Mica/show. Do not re-run after Mica:
/// `set_decorations` / `SetWindowLong`+`FRAMECHANGED` can fight the backdrop.
fn ensure_frameless_dock_chrome(window: &WebviewWindow) -> Result<(), String> {
    if let Err(err) = window.set_decorations(false) {
        log::warn!("[win-backdrop] set_decorations(false) failed: {err}");
    }
    if let Err(err) = window.set_shadow(false) {
        log::warn!("[win-backdrop] set_shadow(false) failed: {err}");
    }
    clear_dock_window_title(window);
    assert_transparent_webview_bg(window, true);

    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let caption_bits =
            WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
        log::info!("[win-backdrop] GWL_STYLE=0x{style:08x}");
        let cleaned = style & !caption_bits;
        if cleaned != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, cleaned as isize);
            if let Err(err) = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            ) {
                log::warn!("[win-backdrop] SetWindowPos(FRAMECHANGED) failed: {err}");
            }
            log::info!("[win-backdrop] stripped caption bits → GWL_STYLE=0x{cleaned:08x}");
        }
    }

    match tauri::webview_version() {
        Ok(v) => {
            log::info!("[win-backdrop] webview2_version={v}");
            // WebView2 144.x painted a ghost titlebar on transparent frameless
            // windows; fixed in ~146+. Log a hint when the runtime looks old.
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

/// Light re-assert after first map — title + transparent bg only (no style rewrite).
fn reassert_dock_chrome_after_show(window: &WebviewWindow) {
    clear_dock_window_title(window);
    assert_transparent_webview_bg(window, true);
}

fn apply_dock_mica(window: &WebviewWindow) -> Result<(), String> {
    use window_vibrancy::apply_mica;

    // Always-dark, like macOS HudWindow — not system semantic Mica.
    match apply_mica(window, Some(true)) {
        Ok(()) => {
            log::info!("[win-backdrop] apply_mica(dark=true) ok");
            Ok(())
        }
        Err(e) => {
            log::error!("[win-backdrop] apply_mica failed: {e}");
            Err(e.to_string())
        }
    }
}

/// Drop Mica while the HWND is larger than the CSS pill (hover / menu).
/// Otherwise DWM paints a gray shell in the magnify/menu margins. The pill
/// keeps its CSS tint; idle re-applies Mica under the pill-shaped region.
fn clear_dock_mica(window: &WebviewWindow) -> Result<(), String> {
    use window_vibrancy::clear_mica;

    match clear_mica(window) {
        Ok(()) => {
            log::info!("[win-backdrop] clear_mica ok (region relaxed)");
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

/// Clears the custom region so the full HWND (incl. menu overlay) is visible,
/// and removes Mica so expanded margins stay transparent (no gray shell).
fn clear_window_rgn(window: &WebviewWindow) -> Result<(), String> {
    // Clear backdrop first — otherwise a frame of full-HWND Mica can flash.
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

/// Clips HWND (+ Mica) to the rounded CSS pill. Cleared while a menu overlay
/// is open, menu-hold is set, or the region is relaxed (hover / magnify / tooltip).
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
    // Idle path: Mica only under the pill-shaped region (re-apply after hover clear).
    apply_dock_mica(window)?;

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
    log::info!(
        "[win-backdrop] SetWindowRgn ok dip=({x_dip:.1},{y_dip:.1} {width_dip:.1}x{height_dip:.1}) \
         px=({left},{top})-({right},{bottom}) diameter={diameter} scale={scale:.2} ok#={}",
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

    // Frameless + transparent before Mica/show — strips residual caption bits
    // and logs WebView2 version for ghost-titlebar triage.
    ensure_frameless_dock_chrome(&window)?;

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
    // Title + transparent bg only — do not rewrite Win32 styles after Mica.
    reassert_dock_chrome_after_show(&window);
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
    store_pill_dims(window, width, height);
    store_pill_client_rect(x, y, width, height);

    let icon_size_dip = current_icon_size_dip(window);
    let changed = resize_dock_window_for_pill(window, width, height, icon_size_dip)?;
    if changed {
        // Resize can shift the pill inside the client area — re-measure is
        // the frontend's job on the next frame; still re-apply from the
        // coords we just stored (usually still correct for edge-anchored).
        refresh_windows_backdrop(window)?;
    } else {
        // Idle → Mica + pill clip; hover/menu → clear Mica + full HWND.
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

