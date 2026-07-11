//! Dock window geometry — shared between platforms. Mirrors the formulas in
//! `src/lib/constants.ts` and the macOS implementation in `platform/macos.rs`.

use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::commands::apps::{AppsState, DockItem, MenuOverlaySide, MenuOverlayState};
use crate::commands::settings::{DockAxis, DockPosition, SettingsState};

const BASE_ICON_SIZE_DIP: f64 = 56.0;
const BASE_DOCK_GAP_DIP: f64 = 8.0;
const BASE_DOCK_PADDING_X_DIP: f64 = 20.0;
const BASE_DOCK_PADDING_Y_DIP: f64 = 12.0;
const BASE_ICON_LED_GAP_DIP: f64 = 8.0;
const LED_HEIGHT_DIP: f64 = 3.0;
const DOCK_SEPARATOR_WIDTH_DIP: f64 = 7.0;
pub const MAGNIFY_MAX_SCALE: f64 = 1.4;
const WINDOW_GLOW_BLEED_DIP: f64 = 32.0;
pub const DOCK_EDGE_INSET_DIP: f64 = 8.0;
pub const PILL_CORNER_RADIUS_DIP: f64 = 28.0;
const TOOLTIP_GAP_DIP: f64 = 16.0;
const TOOLTIP_HEIGHT_DIP: f64 = 28.0;
const CONTEXT_MENU_HEIGHT_DIP: f64 = 214.0;
pub const MENU_OVERLAY_GAP_DIP: f64 = 16.0;
pub const BASE_DOCK_PADDING_Y_DIP_PUB: f64 = BASE_DOCK_PADDING_Y_DIP;
pub const BASE_ICON_SIZE_DIP_PUB: f64 = BASE_ICON_SIZE_DIP;

struct SizeMetrics {
    dock_gap_dip: f64,
    dock_padding_x_dip: f64,
    pill_thickness_dip: f64,
    window_thickness_dip: f64,
}

pub fn axis_css_dims(axis: DockAxis, length: f64, thickness: f64) -> (f64, f64) {
    match axis {
        DockAxis::Horizontal => (length, thickness),
        DockAxis::Vertical => (thickness, length),
    }
}

fn pill_far_reserve_dip(icon_size_dip: f64) -> f64 {
    let scale = icon_size_dip / BASE_ICON_SIZE_DIP;
    let dock_padding_y_dip = (BASE_DOCK_PADDING_Y_DIP * scale).round();
    let magnify_height_overflow_dip = (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil();

    magnify_height_overflow_dip
        .max(TOOLTIP_GAP_DIP + TOOLTIP_HEIGHT_DIP)
        .max(TOOLTIP_GAP_DIP + CONTEXT_MENU_HEIGHT_DIP)
        - dock_padding_y_dip
}

fn window_thickness_dip(pill_thickness_dip: f64, icon_size_dip: f64) -> f64 {
    DOCK_EDGE_INSET_DIP + pill_thickness_dip + pill_far_reserve_dip(icon_size_dip)
}

fn magnify_height_overflow_dip(icon_size_dip: f64) -> f64 {
    (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil()
}

/// Windows resting window thickness: pill + magnify overflow only — no
/// tooltip/menu stack baked in (those grow via `ensure_window_fits_menu_overlay`).
/// Avoids the asymmetric `pill_far_reserve_dip` halo that would skew Mica.
pub fn window_thickness_rest_dip(pill_thickness_dip: f64, icon_size_dip: f64) -> f64 {
    let scale = icon_size_dip / BASE_ICON_SIZE_DIP;
    let dock_padding_y_dip = (BASE_DOCK_PADDING_Y_DIP * scale).round();
    DOCK_EDGE_INSET_DIP + pill_thickness_dip + magnify_height_overflow_dip(icon_size_dip)
        - dock_padding_y_dip
}

fn size_metrics(icon_size_dip: f64, position: DockPosition) -> SizeMetrics {
    let scale = icon_size_dip / BASE_ICON_SIZE_DIP;
    let dock_gap_dip = (BASE_DOCK_GAP_DIP * scale).round();
    let dock_padding_x_dip = (BASE_DOCK_PADDING_X_DIP * scale).round();
    let dock_padding_y_dip = (BASE_DOCK_PADDING_Y_DIP * scale).round();
    let icon_led_gap_dip = (BASE_ICON_LED_GAP_DIP * scale).round();

    let led_along_thickness = matches!(position, DockPosition::Bottom | DockPosition::Top);
    let pill_thickness_dip = if led_along_thickness {
        dock_padding_y_dip * 2.0 + icon_size_dip + icon_led_gap_dip + LED_HEIGHT_DIP
    } else {
        dock_padding_y_dip * 2.0 + icon_size_dip
    };

    SizeMetrics {
        dock_gap_dip,
        dock_padding_x_dip,
        pill_thickness_dip,
        window_thickness_dip: window_thickness_dip(pill_thickness_dip, icon_size_dip),
    }
}

pub fn pill_length_dip(entries: &[DockItem], icon_size_dip: f64, position: DockPosition) -> f64 {
    let metrics = size_metrics(icon_size_dip, position);
    let mut row_length = 0.0;
    for (index, item) in entries.iter().enumerate() {
        if index > 0 {
            row_length += metrics.dock_gap_dip;
        }
        row_length += match item {
            DockItem::App(_) => icon_size_dip,
            DockItem::Separator(_) => DOCK_SEPARATOR_WIDTH_DIP,
        };
    }
    metrics.dock_padding_x_dip * 2.0 + row_length
}

pub fn window_length_dip(pill_length_dip: f64, icon_size_dip: f64) -> f64 {
    pill_length_dip + (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil() + WINDOW_GLOW_BLEED_DIP
}

pub fn sync_icon_size_preview(window: &WebviewWindow, icon_size_dip: f64) {
    let clamped = icon_size_dip.round().clamp(44.0, 72.0);
    let settings_state = window.state::<SettingsState>();
    let mut guard = settings_state
        .preview_icon_size_px
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(clamped);
}

pub fn current_icon_size_dip(window: &WebviewWindow) -> f64 {
    let state = window.state::<SettingsState>();
    {
        let preview = state
            .preview_icon_size_px
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(px) = *preview {
            return px;
        }
    }
    let guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.icon_size_px
}

pub fn current_dock_position(window: &WebviewWindow) -> DockPosition {
    let state = window.state::<SettingsState>();
    let guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.dock_position
}

/// Sizes the window inner DIP and anchors it to the configured screen edge.
pub fn apply_dock_window_frame(
    window: &WebviewWindow,
    target_content_width: f64,
    target_content_height: f64,
    position: DockPosition,
) -> Result<bool, String> {
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no primary monitor".to_string())?;

    let scale = monitor.scale_factor();
    let monitor_size = *monitor.size();
    let monitor_pos = *monitor.position();

    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let current_width = inner.width as f64 / scale;
    let current_height = inner.height as f64 / scale;
    let size_changed = (current_width - target_content_width).abs() >= 0.5
        || (current_height - target_content_height).abs() >= 0.5;

    let width_px = (target_content_width * scale).round() as i32;
    let height_px = (target_content_height * scale).round() as i32;

    if size_changed {
        window
            .set_size(PhysicalSize::new(width_px as u32, height_px as u32))
            .map_err(|e| e.to_string())?;
    }

    let (x, y) = match position {
        DockPosition::Bottom => (
            monitor_pos.x + (monitor_size.width as i32 - width_px) / 2,
            monitor_pos.y + monitor_size.height as i32 - height_px,
        ),
        DockPosition::Top => (
            monitor_pos.x + (monitor_size.width as i32 - width_px) / 2,
            monitor_pos.y,
        ),
        DockPosition::Left => (
            monitor_pos.x,
            monitor_pos.y + (monitor_size.height as i32 - height_px) / 2,
        ),
        DockPosition::Right => (
            monitor_pos.x + monitor_size.width as i32 - width_px,
            monitor_pos.y + (monitor_size.height as i32 - height_px) / 2,
        ),
    };

    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;

    Ok(size_changed)
}

pub fn store_pill_dims(window: &WebviewWindow, pill_width: f64, pill_height: f64) {
    let state = window.state::<AppsState>();
    {
        let mut current_width = state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_width = pill_width;
    }
    {
        let mut current_height = state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_height = pill_height;
    }
}

pub fn resize_dock_window_for_pill(
    window: &WebviewWindow,
    pill_width: f64,
    pill_height: f64,
    icon_size_dip: f64,
) -> Result<bool, String> {
    sync_icon_size_preview(window, icon_size_dip);

    let position = current_dock_position(window);
    let (pill_length, pill_thickness) = axis_css_dims(position.axis(), pill_width, pill_height);
    let window_length = window_length_dip(pill_length, icon_size_dip);
    #[cfg(windows)]
    let window_thickness = window_thickness_rest_dip(pill_thickness, icon_size_dip);
    #[cfg(not(windows))]
    let window_thickness = window_thickness_dip(pill_thickness, icon_size_dip);
    let (target_width, target_height) =
        axis_css_dims(position.axis(), window_length, window_thickness);
    let changed = apply_dock_window_frame(window, target_width, target_height, position)?;
    store_pill_dims(window, pill_width, pill_height);
    Ok(changed)
}

pub fn formula_window_frame(
    entries: &[DockItem],
    icon_size_dip: f64,
    position: DockPosition,
) -> (f64, f64, f64, f64) {
    let pill_length = pill_length_dip(entries, icon_size_dip, position);
    let metrics = size_metrics(icon_size_dip, position);
    let (pill_width, pill_height) =
        axis_css_dims(position.axis(), pill_length, metrics.pill_thickness_dip);
    let window_length = window_length_dip(pill_length, icon_size_dip);
    let (window_width, window_height) =
        axis_css_dims(position.axis(), window_length, metrics.window_thickness_dip);
    (pill_width, pill_height, window_width, window_height)
}

/// Windows hybrid resting frame — pill-centric thickness without tooltip/menu
/// reserve; magnify overflow is static (no hover window resize).
pub fn formula_window_frame_rest(
    entries: &[DockItem],
    icon_size_dip: f64,
    position: DockPosition,
) -> (f64, f64, f64, f64) {
    let pill_length = pill_length_dip(entries, icon_size_dip, position);
    let metrics = size_metrics(icon_size_dip, position);
    let (pill_width, pill_height) =
        axis_css_dims(position.axis(), pill_length, metrics.pill_thickness_dip);
    let window_length = window_length_dip(pill_length, icon_size_dip);
    let rest_thickness = window_thickness_rest_dip(metrics.pill_thickness_dip, icon_size_dip);
    let (window_width, window_height) =
        axis_css_dims(position.axis(), window_length, rest_thickness);
    (pill_width, pill_height, window_width, window_height)
}

pub fn menu_overlay_axis_extents(
    position: DockPosition,
    side: MenuOverlaySide,
    width_dip: f64,
    height_dip: f64,
) -> (f64, f64) {
    if !side.is_active() {
        return (0.0, 0.0);
    }
    match position.axis() {
        DockAxis::Horizontal => match side {
            MenuOverlaySide::Top | MenuOverlaySide::Bottom => {
                (MENU_OVERLAY_GAP_DIP + height_dip, 0.0)
            }
            MenuOverlaySide::Left | MenuOverlaySide::Right => (0.0, MENU_OVERLAY_GAP_DIP + width_dip),
            MenuOverlaySide::None => (0.0, 0.0),
        },
        DockAxis::Vertical => match side {
            MenuOverlaySide::Left | MenuOverlaySide::Right => {
                (MENU_OVERLAY_GAP_DIP + width_dip, 0.0)
            }
            MenuOverlaySide::Top | MenuOverlaySide::Bottom => {
                (0.0, MENU_OVERLAY_GAP_DIP + height_dip)
            }
            MenuOverlaySide::None => (0.0, 0.0),
        },
    }
}

pub fn pill_thickness_hover_dip(pill_thickness_rest_dip: f64, icon_size_dip: f64) -> f64 {
    let scale = icon_size_dip / BASE_ICON_SIZE_DIP;
    let dock_padding_y_dip = (BASE_DOCK_PADDING_Y_DIP * scale).round();
    let magnify_height_overflow_dip = (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil();
    pill_thickness_rest_dip + magnify_height_overflow_dip - dock_padding_y_dip
}

/// Grows the dock window when an open context menu exceeds the current far reserve.
pub fn ensure_window_fits_menu_overlay(
    window: &WebviewWindow,
    overlay: MenuOverlayState,
) -> Result<(), String> {
    if !overlay.is_active() {
        return Ok(());
    }

    let icon_size_dip = current_icon_size_dip(window);
    let position = current_dock_position(window);
    let (pill_length_dip, pill_thickness_dip) = {
        let state = window.state::<AppsState>();
        let stored_width = *state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored_height = *state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (length, thickness) = axis_css_dims(position.axis(), stored_width, stored_height);
        if thickness > 0.0 {
            (length, thickness)
        } else {
            let metrics = size_metrics(icon_size_dip, position);
            (length, metrics.pill_thickness_dip)
        }
    };

    let (menu_thickness_ext, menu_length_ext) = menu_overlay_axis_extents(
        position,
        overlay.side,
        overlay.width_dip,
        overlay.height_dip,
    );

    let scale = icon_size_dip / BASE_ICON_SIZE_DIP;
    let dock_padding_y_dip = (BASE_DOCK_PADDING_Y_DIP * scale).round();
    let magnify_height_overflow_dip = (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil();
    let menu_stack_on_thickness = if menu_thickness_ext > 0.0 {
        menu_thickness_ext
    } else {
        MENU_OVERLAY_GAP_DIP + overlay.height_dip
    };
    let far_reserve_dip = magnify_height_overflow_dip
        .max(TOOLTIP_GAP_DIP + TOOLTIP_HEIGHT_DIP)
        .max(menu_stack_on_thickness)
        - dock_padding_y_dip;
    let target_thickness_dip = DOCK_EDGE_INSET_DIP + pill_thickness_dip + far_reserve_dip;
    let target_length_dip = window_length_dip(pill_length_dip, icon_size_dip) + menu_length_ext;

    let scale_factor = window.scale_factor().map_err(|e| e.to_string())?;
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let current_width_dip = inner.width as f64 / scale_factor;
    let current_height_dip = inner.height as f64 / scale_factor;
    let (current_length_dip, current_thickness_dip) =
        axis_css_dims(position.axis(), current_width_dip, current_height_dip);

    let final_length_dip = current_length_dip.max(target_length_dip);
    let final_thickness_dip = current_thickness_dip.max(target_thickness_dip);

    if final_length_dip <= current_length_dip + 0.5
        && final_thickness_dip <= current_thickness_dip + 0.5
    {
        return Ok(());
    }

    let (target_width_dip, target_height_dip) =
        axis_css_dims(position.axis(), final_length_dip, final_thickness_dip);
    apply_dock_window_frame(window, target_width_dip, target_height_dip, position)?;
    Ok(())
}
