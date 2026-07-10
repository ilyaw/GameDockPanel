//! Windows dock input — click-through poller and low-level mouse hook.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Manager, WebviewWindow};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, SetWindowsHookExW, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP,
};

use crate::commands::apps::AppsState;
use crate::commands::settings::DockPosition;
use crate::platform::geometry::{
    axis_css_dims, current_dock_position, current_icon_size_dip, menu_overlay_axis_extents,
    pill_thickness_hover_dip, DOCK_EDGE_INSET_DIP, PILL_CORNER_RADIUS_DIP,
};

const CLICK_POLL_MS: u64 = 50;
const CLICK_MOVE_TOLERANCE_SQ: i32 = 12 * 12;
const DOUBLE_CLICK_INTERVAL_MS: u64 = 400;

static HOOK_STATE: OnceLock<Arc<HookState>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct DockCursorPayload {
    x: f64,
    y: f64,
}

struct PendingDown {
    x: i32,
    y: i32,
}

struct LastTap {
    at: Instant,
    screen_x: i32,
    screen_y: i32,
}

struct HookState {
    window: WebviewWindow,
    pending_down: std::sync::Mutex<Option<PendingDown>>,
    last_tap: std::sync::Mutex<Option<LastTap>>,
}

pub fn start_dock_input(window: WebviewWindow) {
    start_click_through_poller(window.clone());
    start_dock_click_hook(window);
}

fn start_click_through_poller(window: WebviewWindow) {
    std::thread::spawn(move || {
        let mut ignoring = true;
        let mut dock_hovered = false;
        let mut last_cursor: Option<DockCursorPayload> = None;

        loop {
            std::thread::sleep(Duration::from_millis(CLICK_POLL_MS));

            let Ok(cursor) = window.cursor_position() else {
                continue;
            };
            let cursor_x = cursor.x.round() as i32;
            let cursor_y = cursor.y.round() as i32;

            let pill_cursor =
                pill_cursor_at_screen(&window, cursor_x, cursor_y, dock_hovered);
            let in_pill = pill_cursor.is_some();

            if in_pill != dock_hovered {
                dock_hovered = in_pill;
                let _ = window.emit("dock-hover", dock_hovered);
                if !dock_hovered {
                    last_cursor = None;
                }
            }

            if let Some(cursor) = pill_cursor {
                if last_cursor != Some(cursor) {
                    last_cursor = Some(cursor);
                    let _ = window.emit("dock-cursor", cursor);
                }
            }

            let should_ignore = !in_pill;
            if should_ignore != ignoring {
                if window.set_ignore_cursor_events(should_ignore).is_ok() {
                    ignoring = should_ignore;
                }
            }
        }
    });
}

fn start_dock_click_hook(window: WebviewWindow) {
    let state = Arc::new(HookState {
        window,
        pending_down: std::sync::Mutex::new(None),
        last_tap: std::sync::Mutex::new(None),
    });
    let _ = HOOK_STATE.set(Arc::clone(&state));

    std::thread::spawn(move || {
        let module = unsafe {
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
        };
        let module = match module {
            Ok(m) => m,
            Err(err) => {
                log::error!("GetModuleHandleW failed for mouse hook: {err}");
                return;
            }
        };

        let hook = unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), module, 0)
        };

        match hook {
            Ok(h) if !h.is_invalid() => {
                log::info!("dock-click WH_MOUSE_LL hook installed");
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
            _ => {
                log::error!("failed to install WH_MOUSE_LL hook for dock-click");
            }
        }
    });
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let Some(state) = HOOK_STATE.get() else {
        return CallNextHookEx(None, code, wparam, lparam);
    };

    let msg = wparam.0 as u32;

    match msg {
        WM_LBUTTONDOWN => {
            if let Ok((x, y)) = screen_cursor() {
                if let Ok(mut guard) = state.pending_down.lock() {
                    *guard = Some(PendingDown { x, y });
                }
                if let Some(payload) = cursor_to_window_logical(&state.window, x, y) {
                    let _ = state.window.emit("dock-global-mousedown", payload);
                }
            }
        }
        WM_LBUTTONUP => {
            let is_tap = match (screen_cursor(), state.pending_down.lock()) {
                (Ok((up_x, up_y)), Ok(mut guard)) => {
                    let Some(down) = guard.take() else {
                        return CallNextHookEx(None, code, wparam, lparam);
                    };
                    let dx = up_x - down.x;
                    let dy = up_y - down.y;
                    dx * dx + dy * dy <= CLICK_MOVE_TOLERANCE_SQ
                }
                _ => false,
            };

            if is_tap {
                if let Ok((cursor_x, cursor_y)) = screen_cursor() {
                    if let Some(payload) =
                        pill_cursor_at_screen(&state.window, cursor_x, cursor_y, true)
                    {
                        let scale = state.window.scale_factor().unwrap_or(1.0);
                        let icon_tol = (current_icon_size_dip(&state.window) * scale * 1.5)
                            .round()
                            .max(24.0) as i32;
                        let is_double = match state.last_tap.lock() {
                            Ok(mut guard) => {
                                let now = Instant::now();
                                let double = guard.as_ref().is_some_and(|prev| {
                                    now.duration_since(prev.at)
                                        < Duration::from_millis(DOUBLE_CLICK_INTERVAL_MS)
                                        && (prev.screen_x - cursor_x).abs() <= icon_tol
                                        && (prev.screen_y - cursor_y).abs() <= icon_tol
                                });
                                if double {
                                    *guard = None;
                                } else {
                                    *guard = Some(LastTap {
                                        at: now,
                                        screen_x: cursor_x,
                                        screen_y: cursor_y,
                                    });
                                }
                                double
                            }
                            Err(_) => false,
                        };
                        let _ = state.window.emit("dock-click", payload);
                        if is_double {
                            let _ = state.window.emit("dock-double-click", payload);
                        }
                    }
                }
            }
        }
        _ => {}
    }

    CallNextHookEx(None, code, wparam, lparam)
}

fn screen_cursor() -> Result<(i32, i32), ()> {
    use windows::Win32::Foundation::POINT;
    let mut pt = POINT::default();
    unsafe {
        GetCursorPos(&mut pt).map_err(|_| ())?;
    }
    Ok((pt.x, pt.y))
}

fn cursor_to_window_logical(
    window: &WebviewWindow,
    screen_x: i32,
    screen_y: i32,
) -> Option<DockCursorPayload> {
    let scale = window.scale_factor().ok()?;
    let outer_pos = window.outer_position().ok()?;
    Some(DockCursorPayload {
        x: (screen_x - outer_pos.x) as f64 / scale,
        y: (screen_y - outer_pos.y) as f64 / scale,
    })
}

fn pill_cursor_at_screen(
    window: &WebviewWindow,
    screen_x: i32,
    screen_y: i32,
    dock_hovered: bool,
) -> Option<DockCursorPayload> {
    let scale = window.scale_factor().ok()?;
    let outer_pos = window.outer_position().ok()?;
    let outer_size = window.outer_size().ok()?;
    let position = current_dock_position(window);

    let (pill_length_dip, pill_thickness_dip) =
        pill_hit_dims_for_cursor(window, dock_hovered);

    let (pill_w_dip, pill_h_dip) =
        axis_css_dims(position.axis(), pill_length_dip, pill_thickness_dip);

    let pill_w = (pill_w_dip * scale).round() as i32;
    let pill_h = (pill_h_dip * scale).round() as i32;
    let inset = (DOCK_EDGE_INSET_DIP * scale).round() as i32;
    let radius = (PILL_CORNER_RADIUS_DIP * scale).round() as i32;

    let (pill_left, pill_top, pill_right, pill_bottom) =
        pill_rect_for_position(position, outer_pos, outer_size, pill_w, pill_h, inset);

    if !in_rounded_rect(
        screen_x,
        screen_y,
        pill_left,
        pill_top,
        pill_right,
        pill_bottom,
        radius,
    ) {
        return None;
    }

    Some(DockCursorPayload {
        x: (screen_x - outer_pos.x) as f64 / scale,
        y: (screen_y - outer_pos.y) as f64 / scale,
    })
}

fn pill_hit_dims_for_cursor(window: &WebviewWindow, dock_hovered: bool) -> (f64, f64) {
    let position = current_dock_position(window);
    let icon_size_dip = current_icon_size_dip(window);
    let pill_thickness_rest_dip = current_pill_thickness_rest_dip(window);

    let (rest_length_dip, _) = {
        let state = window.state::<AppsState>();
        let stored_width = *state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored_height = *state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        axis_css_dims(position.axis(), stored_width, stored_height)
    };

    let menu_overlay = {
        let state = window.state::<AppsState>();
        let guard = state
            .menu_overlay
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard
    };

    let (menu_thickness_ext, menu_length_ext) = menu_overlay_axis_extents(
        position,
        menu_overlay.side,
        menu_overlay.width_dip,
        menu_overlay.height_dip,
    );

    let thickness = if menu_overlay.is_active() {
        pill_thickness_rest_dip + menu_thickness_ext
    } else if dock_hovered {
        pill_thickness_hover_dip(pill_thickness_rest_dip, icon_size_dip)
    } else {
        pill_thickness_rest_dip
    };

    let length = if menu_overlay.is_active() && menu_length_ext > 0.0 {
        rest_length_dip + menu_length_ext
    } else {
        rest_length_dip
    };

    (length, thickness)
}

fn current_pill_thickness_rest_dip(window: &WebviewWindow) -> f64 {
    let state = window.state::<AppsState>();
    let stored_width = *state
        .pill_width_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let stored_height = *state
        .pill_height_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let position = current_dock_position(window);
    let (_, thickness) = axis_css_dims(position.axis(), stored_width, stored_height);
    thickness
}

fn pill_rect_for_position(
    position: DockPosition,
    outer_pos: tauri::PhysicalPosition<i32>,
    outer_size: tauri::PhysicalSize<u32>,
    pill_w: i32,
    pill_h: i32,
    inset: i32,
) -> (i32, i32, i32, i32) {
    let (left, top) = match position {
        DockPosition::Bottom => (
            outer_pos.x + (outer_size.width as i32 - pill_w) / 2,
            outer_pos.y + outer_size.height as i32 - inset - pill_h,
        ),
        DockPosition::Top => (
            outer_pos.x + (outer_size.width as i32 - pill_w) / 2,
            outer_pos.y + inset,
        ),
        DockPosition::Left => (
            outer_pos.x + inset,
            outer_pos.y + (outer_size.height as i32 - pill_h) / 2,
        ),
        DockPosition::Right => (
            outer_pos.x + outer_size.width as i32 - inset - pill_w,
            outer_pos.y + (outer_size.height as i32 - pill_h) / 2,
        ),
    };
    (left, top, left + pill_w, top + pill_h)
}

fn in_rounded_rect(
    x: i32,
    y: i32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
) -> bool {
    if x < left || x > right || y < top || y > bottom {
        return false;
    }

    let corner_x = if x < left + radius {
        left + radius
    } else if x > right - radius {
        right - radius
    } else {
        return true;
    };

    let corner_y = if y < top + radius {
        top + radius
    } else if y > bottom - radius {
        bottom - radius
    } else {
        return true;
    };

    let dx = x - corner_x;
    let dy = y - corner_y;
    dx * dx + dy * dy <= radius * radius
}
