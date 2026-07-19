//! Windows dock input — click-through poller only.
//!
//! ## Why there is no `WH_MOUSE_LL` hook
//!
//! An earlier skeleton installed a global low-level mouse hook to mirror
//! macOS `CGEventTap` (`dock-click` / `dock-global-mousedown`). That pattern
//! does not apply on Windows: WebView2 does not swallow `mouseDown` the way
//! `NSVisualEffectView` can, and the hook thread had no Win32 message loop
//! (`GetMessage`), so the callback was unreliable anyway.
//!
//! Pill clicks are handled by the WebView when the click-through poller clears
//! `WS_EX_TRANSPARENT` over the rounded pill hit-test (native exstyle toggle —
//! not Tao `set_ignore_cursor_events`, which restores `WS_CAPTION`); see
//! `DockPanel.tsx` native pointer handlers on Windows. Global outside-window
//! menu dismiss (macOS tap) is not replicated here — in-window dismiss still
//! works via DOM listeners on `DockIcon`.
//!
//! Deliberately no global hook: `WH_MOUSE_LL` is flagged by game anti-cheats
//! (Vanguard/EAC/BattlEye), which matters for this product's audience.

use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager, WebviewWindow};

use crate::commands::apps::AppsState;
use crate::commands::settings::DockPosition;
use crate::platform::geometry::{
    axis_css_dims, current_dock_position, current_icon_size_dip, menu_overlay_axis_extents,
    pill_thickness_hover_dip, PILL_CORNER_RADIUS_DIP,
};

use super::window::set_dock_click_through;

const CLICK_POLL_MS: u64 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct DockCursorPayload {
    x: f64,
    y: f64,
}

pub fn start_dock_input(window: WebviewWindow) {
    start_click_through_poller(window);
}

fn start_click_through_poller(window: WebviewWindow) {
    std::thread::spawn(move || {
        let mut ignoring = true;
        let mut dock_hovered = false;
        let mut last_cursor: Option<DockCursorPayload> = None;

        // Force initial TRANSPARENT|LAYERED on the UI thread — setup may have
        // raced with Tao `show`/`always_on_top`, and `ignoring=true` alone
        // would skip the first toggle.
        if let Err(err) = set_dock_click_through(&window, true) {
            log::warn!("[win-backdrop] initial click-through failed: {err}");
        }

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
            let in_window = cursor_in_window_bounds(&window, cursor_x, cursor_y);
            // Explorer drag-drop needs the WebView to receive OLE events — while
            // click-through (`WS_EX_TRANSPARENT`) is on, drops never reach
            // `onDragDropEvent`. Briefly accept hits over the whole window while
            // the left button is held (typical external file drag).
            let lbutton_down = unsafe {
                use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
                GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0
            };
            let drag_over_window = lbutton_down && in_window;

            if in_pill != dock_hovered {
                dock_hovered = in_pill;
                let _ = window.emit("dock-hover", dock_hovered);
                // Magnify + tooltip paint outside the CSS pill — expand the
                // HWND while hovered, shrink when idle.
                if let Err(err) =
                    crate::platform::set_dock_region_relaxed(&window, dock_hovered, None)
                {
                    log::warn!("[win-backdrop] hover region_relaxed failed: {err}");
                }
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

            let should_ignore = !in_pill && !drag_over_window;
            if should_ignore != ignoring {
                match set_dock_click_through(&window, should_ignore) {
                    Ok(()) => ignoring = should_ignore,
                    Err(err) => {
                        log::warn!("[win-backdrop] click-through toggle failed: {err}")
                    }
                }
            }
        }
    });
}

fn cursor_in_window_bounds(window: &WebviewWindow, screen_x: i32, screen_y: i32) -> bool {
    let Ok(outer_pos) = window.outer_position() else {
        return false;
    };
    let Ok(outer_size) = window.outer_size() else {
        return false;
    };
    let left = outer_pos.x;
    let top = outer_pos.y;
    let right = left + outer_size.width as i32;
    let bottom = top + outer_size.height as i32;
    screen_x >= left && screen_x <= right && screen_y >= top && screen_y <= bottom
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
    // Near-edge inset is outside the HWND (see geometry.rs) — pill is flush
    // to the near client edge; do not offset hit-test by DOCK_EDGE_INSET.
    let radius = (PILL_CORNER_RADIUS_DIP * scale).round() as i32;

    let (pill_left, pill_top, pill_right, pill_bottom) =
        pill_rect_for_position(position, outer_pos, outer_size, pill_w, pill_h, 0);

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
