use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{App, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::commands::apps::{AppsState, DockItem, MenuOverlaySide, MenuOverlayState};
use crate::commands::settings::{DockAxis, DockPosition, DockWindowLayer, SettingsState};
use crate::platform::IconResolveResult;

/// Cursor position in webview logical (DIP) coords — emitted while the pointer
/// is over the dock pill so React can hit-test icons without CSS :hover.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct DockCursorPayload {
    x: f64,
    y: f64,
}

// Layout formula mirrored from src/lib/constants.ts — see getSizeMetrics()/
// pillWidthPx()/windowWidthDip() there for the JS-side copy of this math.
// DIP == points throughout (Tauri's logical-pixel convention), so these are
// used directly against `NSRect`/`NSWindow` without any extra unit
// conversion. Everything that scales with icon size is derived by
// `size_metrics()` from a single `icon_size_dip` input (see
// `PROMPT_11_ADJUSTABLE_HEIGHT.md`) instead of being a fixed constant.

/// Mirrors `ICON_SIZE_PRESETS` in src/lib/constants.ts — id comes from
/// Reads the current icon size straight from `SettingsState` — used by every
/// geometry call site below instead of threading an extra parameter through
/// each one (mirrors how `pill_width_dip`/`pill_height_dip` are already read
/// from `AppsState` rather than passed around).
fn icon_export_px(icon_size_dip: f64, scale_factor: f64) -> f64 {
    (icon_size_dip * MAGNIFY_MAX_SCALE * scale_factor)
        .ceil()
        .clamp(ICON_EXPORT_MIN_PX, ICON_EXPORT_MAX_PX)
}

/// Records the live (preview/spring) icon size into its own
/// `SettingsState.preview_icon_size_px` slot — NOT into
/// `settings.icon_size_px`. Writing the persisted field here used to make
/// `apply_dock_settings`'s before/after comparison see two equal values by
/// commit time, silently skipping `refresh_dock_icons` after slider-driven
/// size changes (see the field's doc comment in commands/settings.rs).
fn sync_icon_size_preview(window: &WebviewWindow, icon_size_dip: f64) {
    let clamped = icon_size_dip.round().clamp(44.0, 72.0);
    let settings_state = window.state::<SettingsState>();
    let mut guard = settings_state
        .preview_icon_size_px
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(clamped);
}

/// Live icon size for geometry/hit-testing: the in-flight preview value
/// when a slider drag / size spring is active, else the committed setting.
fn current_icon_size_dip(window: &WebviewWindow) -> f64 {
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

/// Reads the current `DockPosition` fresh from `SettingsState` — same
/// "state, not a cached/compile-time value" rationale as
/// `current_icon_size_dip`, so a settings-window position change takes
/// effect immediately on the next geometry call, without restarting the
/// click-through poller or click tap threads.
fn current_dock_position(window: &WebviewWindow) -> DockPosition {
    let state = window.state::<SettingsState>();
    let guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.dock_position
}

/// Reads the current `DockWindowLayer` fresh from `SettingsState`.
fn current_dock_window_layer(window: &WebviewWindow) -> DockWindowLayer {
    let state = window.state::<SettingsState>();
    let guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.dock_window_layer
}

/// Applies the dock's z-order — always-on-top vs normal window level.
pub fn apply_dock_window_layer(
    window: &WebviewWindow,
    layer: DockWindowLayer,
) -> Result<(), String> {
    let on_top = matches!(layer, DockWindowLayer::AboveWindows);
    window
        .set_always_on_top(on_top)
        .map_err(|e| e.to_string())
}

/// Maps the orientation-neutral `(length, thickness)` pair onto actual CSS
/// `(width, height)` per `DockPosition::axis` — Bottom/Top keep
/// length→width, thickness→height (the dock's original math); Left/Right
/// swap them. Self-inverse (`axis_css_dims(axis, axis_css_dims(axis, a,
/// b))` reproduces `(a, b)`), so the same helper also converts a measured
/// `(width, height)` pair back into `(length, thickness)`.
fn axis_css_dims(axis: DockAxis, length: f64, thickness: f64) -> (f64, f64) {
    match axis {
        DockAxis::Horizontal => (length, thickness),
        DockAxis::Vertical => (thickness, length),
    }
}

// Reference icon size the constants below were originally tuned against
// (the dock's pre-preset fixed size) — mirrors `BASE_ICON_SIZE_PX` in
// src/lib/constants.ts.
const BASE_ICON_SIZE_DIP: f64 = 56.0;
const BASE_DOCK_GAP_DIP: f64 = 8.0;
const BASE_DOCK_PADDING_X_DIP: f64 = 20.0;
const BASE_DOCK_PADDING_Y_DIP: f64 = 12.0;
const BASE_ICON_LED_GAP_DIP: f64 = 8.0;
const LED_HEIGHT_DIP: f64 = 3.0;

// Fixed across every icon-size preset — see the "stays fixed" rationale in
// src/lib/constants.ts next to the JS copies of these same constants.
/// Horizontal slot for an in-row dock separator — mirrors
/// `DOCK_SEPARATOR_WIDTH_DIP` in src/lib/constants.ts.
const DOCK_SEPARATOR_WIDTH_DIP: f64 = 7.0;
const MAGNIFY_MAX_SCALE: f64 = 1.4;
const WINDOW_GLOW_BLEED_DIP: f64 = 32.0;
/// Gap between the dock pill's near edge and the screen edge it's anchored
/// to — bottom edge for `DockPosition::Bottom`, top for `Top`, etc. Named
/// generically since Phase 1 (PROMPT_15_POSITION_PHASE1.md) generalized
/// anchoring beyond bottom-only. Mirrors `DOCK_EDGE_INSET_PX` in
/// src/lib/constants.ts.
const DOCK_EDGE_INSET_DIP: f64 = 8.0;
/// Must match Tailwind's `rounded-[28px]` on the dock pill (DockPanel.tsx) —
/// CSS and the native vibrancy/hit-test masks below only agree with the
/// visible shape if this stays in sync with that class.
const PILL_CORNER_RADIUS_DIP: f64 = 28.0;
/// Mirrors `TOOLTIP_GAP_PX` in src/lib/constants.ts.
const TOOLTIP_GAP_DIP: f64 = 16.0;
/// Mirrors `TOOLTIP_HEIGHT_PX` in src/lib/constants.ts.
const TOOLTIP_HEIGHT_DIP: f64 = 28.0;
/// Mirrors `CONTEXT_MENU_HEIGHT_PX` in src/lib/constants.ts — reserve for the
/// tallest footprint (main column or open «Параметры» submenu + dividers).
const CONTEXT_MENU_HEIGHT_DIP: f64 = 214.0;
const CLICK_POLL_MS: u64 = 50;
/// Rare backup for apps that never post `didTerminate` (e.g. Voice Memos).
const RUNNING_RECONCILE_POLL_MS: u64 = 4000;
/// Post-`open_path` retries when `didLaunch` is late or missing — each tick
/// re-queries `NSRunningApplication` on the main thread until the bundle is
/// live or the schedule is exhausted.
const LAUNCH_WATCH_DELAYS_MS: &[u64] = &[0, 150, 300, 600, 1200, 2400];
/// Matches `NSEvent.doubleClickInterval` (~500 ms on macOS).
const DOUBLE_CLICK_INTERVAL_MS: u64 = 500;
/// Visual gap between a zoomed window edge and the dock pill — mirrors the
/// small breathing room macOS leaves above the system Dock.
const ZOOM_DOCK_GAP_PX: i32 = 4;
/// Frame comparison tolerance when deciding whether a window is already zoomed.
const ZOOM_FRAME_TOLERANCE_PX: i32 = 4;
/// Mirrors `TOOLTIP_GAP_PX` in src/lib/constants.ts — the gap between the
/// open context menu's own bottom edge and the icon it hangs off of. Used
/// to extend the click-through hit-test up through that gap and into the
/// menu itself (see `AppsState::menu_overlay`).
const MENU_OVERLAY_GAP_DIP: f64 = 16.0;

/// How far a menu extends along the dock thickness vs length axis, given
/// which side of the anchor icon it opens on.
fn menu_overlay_axis_extents(
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

/// Hit-test pill length/thickness in DIP, including open-menu extensions.
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

/// Raster export cap for native icon PNGs — sized from icon display metrics
/// via `icon_export_px`, not a fixed constant.
const ICON_EXPORT_MAX_PX: f64 = 512.0;
const ICON_EXPORT_MIN_PX: f64 = 128.0;

/// Mirrors `SizeMetrics`/`getSizeMetrics` in src/lib/constants.ts — trimmed
/// to just the fields Rust itself reads (window sizing / hit-testing).
/// Magnify curve numbers (influence radius, etc.) stay JS-only since Rust
/// never renders the magnify animation.
///
/// `*_thickness_dip` names the dock's fixed, icon-size-driven axis — CSS
/// height for `DockPosition::Bottom`/`Top`, CSS width for `Left`/`Right`
/// (see `axis_css_dims`).
struct SizeMetrics {
    dock_gap_dip: f64,
    dock_padding_x_dip: f64,
    pill_thickness_dip: f64,
    window_thickness_dip: f64,
}

/// Transparent band on the far side of the pill (away from the anchored
/// screen edge) inside the window — big enough for whichever thing
/// currently pokes furthest past it (magnified icon, hover tooltip, or the
/// taller context menu). Mirrors `pillFarReservePx` in `getSizeMetrics`
/// (src/lib/constants.ts).
///
/// Named for history (this dock only ever anchored to the bottom when it
/// was introduced) — still literally "above the pill" for
/// `DockPosition::Bottom`/`Top`, but for `Left`/`Right` this reserve maps
/// onto the far side of the *thickness* axis (screen-horizontal) — the
/// direction magnify/tooltip/menu now grow after Phase 2.
fn pill_far_reserve_dip(icon_size_dip: f64) -> f64 {
    let scale = icon_size_dip / BASE_ICON_SIZE_DIP;
    let dock_padding_y_dip = (BASE_DOCK_PADDING_Y_DIP * scale).round();
    let magnify_height_overflow_dip = (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil();

    magnify_height_overflow_dip
        .max(TOOLTIP_GAP_DIP + TOOLTIP_HEIGHT_DIP)
        .max(TOOLTIP_GAP_DIP + CONTEXT_MENU_HEIGHT_DIP)
        - dock_padding_y_dip
}

/// Window size along the thickness axis for a given (measured or formula)
/// pill thickness — mirrors `windowThicknessDip` derivation in
/// `getSizeMetrics`. Takes the pill thickness as a parameter (like
/// `window_length_dip` takes pill length) so the primary, DOM-measured
/// resize path can feed in the real rendered pill thickness while only the
/// invisible far-reserve margin comes from the formula. Maps onto CSS
/// height for `DockPosition::Bottom`/`Top`, CSS width for `Left`/`Right`.
fn window_thickness_dip(pill_thickness_dip: f64, icon_size_dip: f64) -> f64 {
    DOCK_EDGE_INSET_DIP + pill_thickness_dip + pill_far_reserve_dip(icon_size_dip)
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

/// Mirrors `pillThicknessHoverPx` in `getSizeMetrics` — used by the
/// click-tap and click-through hit-test, which read the *measured* rest
/// thickness from `AppsState` and only need the formula-only
/// magnify-overflow margin added on top (same "measured base + formula
/// margin" pattern as `window_thickness_dip`). Grows the thickness
/// dimension away from the near edge — see `pill_rect_for_position`.
fn pill_thickness_hover_dip(pill_thickness_rest_dip: f64, icon_size_dip: f64) -> f64 {
    let magnify_height_overflow_dip = (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil();
    pill_thickness_rest_dip + magnify_height_overflow_dip
}

/// Pill size along the length axis (grows/shrinks with item count) — maps
/// onto CSS width for `DockPosition::Bottom`/`Top`, CSS height for
/// `Left`/`Right`. Mirrors `pillLengthPx` in src/lib/constants.ts.
fn pill_length_dip(entries: &[DockItem], icon_size_dip: f64, position: DockPosition) -> f64 {
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

/// Window size along the length axis: the pill's own length plus room for
/// endmost icons bulging outward on magnify, plus RGB-glow bleed. Mirrors
/// `windowLengthDip` in src/lib/constants.ts.
fn window_length_dip(pill_length_dip: f64, icon_size_dip: f64) -> f64 {
    pill_length_dip + (icon_size_dip * (MAGNIFY_MAX_SCALE - 1.0)).ceil() + WINDOW_GLOW_BLEED_DIP
}

/// Positions, sizes and reveals the main window: a compact, always-on-top
/// strip anchored to whichever screen edge `DockPosition` currently
/// selects (see the anchoring model in PROMPT_15_POSITION_PHASE1.md), with
/// the app hidden from the Dock. Initial length is computed from
/// `AppsState.entries` (populated by `commands::apps::init_entries` just
/// before this runs), not a fixed constant.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let icon_size_dip = current_icon_size_dip(&window);
    let position = current_dock_position(&window);
    let entries = app.state::<AppsState>().entries_snapshot();
    let pill_length = pill_length_dip(&entries, icon_size_dip, position);
    let metrics = size_metrics(icon_size_dip, position);
    let (pill_width, pill_height) =
        axis_css_dims(position.axis(), pill_length, metrics.pill_thickness_dip);
    let window_length = window_length_dip(pill_length, icon_size_dip);
    let (window_width, window_height) =
        axis_css_dims(position.axis(), window_length, metrics.window_thickness_dip);

    apply_dock_window_frame(&window, window_width, window_height, position)?;
    apply_dock_window_layer(&window, current_dock_window_layer(&window))?;

    // Pass clicks through everywhere except the pill hitbox (poller toggles).
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;

    {
        let state = app.state::<AppsState>();
        let mut current_width = state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_width = pill_width;
        let mut current_height = state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_height = pill_height;
    }

    enable_inactive_mouse_tracking(&window)?;
    apply_dock_vibrancy(&window, pill_width, pill_height, position)?;

    start_dock_click_tap(window.clone());

    // Reveal immediately with the formula-sized frame; the WebView's first
    // `sync_vibrancy_pill` call then aligns the blur mask to the measured
    // CSS pill. Without this fallback the window stays `visible: false` in
    // tauri.conf.json until that sync runs — if the first DOM measure is
    // delayed (Motion spring values, Strict Mode, etc.) the dock never appears.
    window.show().map_err(|e| e.to_string())?;

    start_click_through_poller(window);

    Ok(())
}

/// `window_vibrancy`'s internal `NSView` tag for the blur view it creates —
/// not exported by the crate (see `NS_VIEW_TAG_BLUR_VIEW` in window-vibrancy
/// 0.7.1's `src/macos/vibrancy.rs`). Duplicated here only to look that view
/// back up after `apply_vibrancy()` so we can resize its frame.
#[cfg(target_os = "macos")]
const VIBRANCY_VIEW_TAG: isize = 91_376_254;

/// Applies native macOS blur behind the dock and masks it to the pill's
/// rounded footprint at the given initial width. `apply_vibrancy()` always
/// sizes its `NSVisualEffectView` to the whole window content view, which is
/// bigger than the visible pill — the extra space is reserved for RGB-glow
/// bleed and magnify/tooltip overflow (see WINDOW_*_DIP vs PILL_*_DIP
/// above). Left untouched, vibrancy would paint a rectangular blur halo
/// outside the pill's `rounded-[28px]` corners. So after applying vibrancy
/// we look the blur view back up by its internal tag and shrink its frame
/// down to just the pill's rect — the crate's own `radius` argument then
/// rounds that smaller rect correctly.
#[cfg(target_os = "macos")]
fn apply_dock_vibrancy(
    window: &WebviewWindow,
    pill_width_dip: f64,
    pill_height_dip: f64,
    position: DockPosition,
) -> Result<(), String> {
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};

    // HudWindow stays dark unconditionally (a fixed-style material, not a
    // semantic one) — matches the always-dark Razer-Chroma look regardless
    // of the user's macOS appearance setting. Sidebar is semantic and
    // resolves light/translucent in Light Mode, which would wash out the
    // RGB frame and bg-zinc-950/80 tint.
    apply_vibrancy(
        window,
        NSVisualEffectMaterial::HudWindow,
        None,
        Some(PILL_CORNER_RADIUS_DIP),
    )
    .map_err(|e| e.to_string())?;

    set_vibrancy_pill_frame(window, pill_width_dip, pill_height_dip, None, None, position)
}

/// Startup fallback pill origin (window-local coords) before the first DOM
/// measurement lands — see `sync_vibrancy_pill_from_web` for the
/// steady-state path that measures directly instead. Mirrors
/// `apply_dock_window_frame`'s screen-level near-edge/centered anchoring
/// model, just expressed relative to the window's own bounds.
#[cfg(target_os = "macos")]
fn fallback_pill_origin(
    position: DockPosition,
    bounds_width: f64,
    bounds_height: f64,
    width: f64,
    height: f64,
    is_flipped: bool,
) -> (f64, f64) {
    // `near_edge_y(from_top)`: the Y origin that puts the pill near the
    // window's top edge (`from_top = true`) or bottom edge (`false`),
    // accounting for `NSView.isFlipped` (flipped: origin top-left, Y grows
    // down; non-flipped: origin bottom-left, Y grows up).
    let near_edge_y = |from_top: bool| {
        if from_top == is_flipped {
            DOCK_EDGE_INSET_DIP
        } else {
            bounds_height - DOCK_EDGE_INSET_DIP - height
        }
    };

    match position {
        DockPosition::Bottom => ((bounds_width - width) / 2.0, near_edge_y(false)),
        DockPosition::Top => ((bounds_width - width) / 2.0, near_edge_y(true)),
        DockPosition::Left => (DOCK_EDGE_INSET_DIP, (bounds_height - height) / 2.0),
        DockPosition::Right => (
            bounds_width - DOCK_EDGE_INSET_DIP - width,
            (bounds_height - height) / 2.0,
        ),
    }
}

/// Snaps a webview-local pill rect to the physical-pixel grid as one unit.
#[cfg(target_os = "macos")]
fn snap_dip_rect(x: f64, y: f64, w: f64, h: f64, scale: f64) -> (f64, f64, f64, f64) {
    if scale <= 0.0 {
        return (x, y, w, h);
    }
    let left = (x * scale).round() / scale;
    let top = (y * scale).round() / scale;
    let right = ((x + w) * scale).round() / scale;
    let bottom = ((y + h) * scale).round() / scale;
    (
        left,
        top,
        (right - left).max(0.0),
        (bottom - top).max(0.0),
    )
}

/// Resizes the masked vibrancy blur view to the given pill footprint.
/// When `origin_x` / `origin_y` are `None`, the frame is anchored via
/// `fallback_pill_origin` (startup before DOM measure). When provided,
/// values are webview logical coords from `getBoundingClientRect()` (see
/// `sync_vibrancy_pill_from_web`).
#[cfg(target_os = "macos")]
fn set_vibrancy_pill_frame(
    window: &WebviewWindow,
    width_dip: f64,
    height_dip: f64,
    origin_x: Option<f64>,
    origin_y: Option<f64>,
    position: DockPosition,
) -> Result<(), String> {
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};

    let ns_view_ptr = window.ns_view().map_err(|e| e.to_string())? as *mut NSView;
    let ns_view = unsafe { &*ns_view_ptr };

    let Some(blur_view) = ns_view.viewWithTag(VIBRANCY_VIEW_TAG) else {
        return Ok(());
    };

    let parent = unsafe {
        blur_view
            .superview()
            .ok_or_else(|| "blur view has no superview".to_string())?
    };
    let bounds = parent.bounds();
    let is_flipped = parent.isFlipped();
    let scale = window.scale_factor().unwrap_or(1.0);

    blur_view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);

    let fallback_origin = fallback_pill_origin(
        position,
        bounds.size.width,
        bounds.size.height,
        width_dip,
        height_dip,
        is_flipped,
    );

    let (local_x, local_y) = match (origin_x, origin_y) {
        (Some(x), Some(y)) if is_flipped => (x, y),
        (Some(x), Some(y)) => (x, bounds.size.height - y - height_dip),
        _ => fallback_origin,
    };

    let (local_x, local_y, local_w, local_h) =
        snap_dip_rect(local_x, local_y, width_dip, height_dip, scale);

    let mut pill_frame = bounds;
    pill_frame.origin.x = local_x;
    pill_frame.origin.y = local_y;
    pill_frame.size.width = local_w;
    pill_frame.size.height = local_h;

    blur_view.setClipsToBounds(true);
    blur_view.setFrame(pill_frame);

    // `apply_vibrancy` sets this once at install time; re-apply after every
    // frame resize so WKWebView geometry sync can't leave a rectangular blur
    // halo peeking through the pill's rounded corners.
    unsafe {
        use objc2::msg_send;
        let _: () = msg_send![&*blur_view, setCornerRadius: PILL_CORNER_RADIUS_DIP];
    }

    Ok(())
}

/// Sizes the window to `target_content_width` × `target_content_height`
/// (inner DIP) and anchors it to the near edge for `position`, centered on
/// the other screen dimension — same geometry as `setup_dock_window`. See
/// the anchoring model in PROMPT_15_POSITION_PHASE1.md. Always updates
/// position even when the size is unchanged, so a prior resize that grew
/// without recentering is corrected on the next call — this is what keeps
/// the dock properly anchored on *every* geometry change (app add/remove,
/// icon-size preset switch, position switch alike), not just the one that
/// happened to change the size. Uses inner size for both `set_size` and
/// the centering math (not `outer_size`) to stay consistent with
/// `setup_dock_window` and avoid macOS inner/outer drift.
#[cfg(target_os = "macos")]
fn apply_dock_window_frame(
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

/// Dispatches `apply_dock_window_frame` onto the main thread — window
/// frame mutations must not run from the Tauri command threadpool.
#[cfg(target_os = "macos")]
fn set_window_frame_instant(
    window: &WebviewWindow,
    target_content_width: f64,
    target_content_height: f64,
    position: DockPosition,
) -> Result<bool, String> {
    let window = window.clone();
    let window_for_closure = window.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let _ = tx.send(apply_dock_window_frame(
                &window_for_closure,
                target_content_width,
                target_content_height,
                position,
            ));
        })
        .map_err(|e| e.to_string())?;

    rx.recv()
        .map_err(|_| "set_window_frame_instant did not complete".to_string())?
}

/// Resizes the native window inner size to fit `pill_width` × `pill_height`
/// (already orientation-correct CSS values, as measured from the DOM) —
/// pill + magnify overflow + glow bleed on the length axis, near-edge
/// inset + far reserve on the thickness axis — re-anchoring it in the same
/// call. Does not touch the vibrancy mask — call `sync_vibrancy_pill_from_web`
/// afterwards once the WebView has laid out.
#[cfg(target_os = "macos")]
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
    let window_thickness = window_thickness_dip(pill_thickness, icon_size_dip);
    let (target_width, target_height) =
        axis_css_dims(position.axis(), window_length, window_thickness);
    let changed = set_window_frame_instant(window, target_width, target_height, position)?;

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

    Ok(changed)
}

/// Restores the native window frame to the formula size for the last
/// DOM-measured pill — called when a context menu closes after
/// `ensure_window_fits_menu_overlay` may have grown the window.
#[cfg(target_os = "macos")]
pub fn shrink_dock_window_to_stored_pill(window: &WebviewWindow) -> Result<bool, String> {
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
        return Ok(false);
    }
    let icon_size_dip = current_icon_size_dip(window);
    resize_dock_window_for_pill(window, pill_width, pill_height, icon_size_dip)
}

/// Grows the dock window when an open context menu exceeds the current far
/// reserve — prevents the menu from being clipped by the webview's
/// `overflow: hidden` boundary. Grows the thickness and/or length axis as
/// needed based on the menu's resolved placement side.
#[cfg(target_os = "macos")]
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
    set_window_frame_instant(window, target_width_dip, target_height_dip, position)?;
    Ok(())
}

/// Formula-based resize when the DOM has not measured yet — kept as a
/// belt-and-suspenders fallback for future call sites.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn resize_dock_window_for_app_count(
    window: &WebviewWindow,
    entries: &[DockItem],
) -> Result<bool, String> {
    let icon_size_dip = current_icon_size_dip(window);
    let position = current_dock_position(window);
    let pill_length = pill_length_dip(entries, icon_size_dip, position);
    let pill_thickness = size_metrics(icon_size_dip, position).pill_thickness_dip;
    let (pill_width, pill_height) = axis_css_dims(position.axis(), pill_length, pill_thickness);
    resize_dock_window_for_pill(window, pill_width, pill_height, icon_size_dip)
}

/// Aligns the masked vibrancy blur view to the pill's measured DOM rect.
/// Does not resize the window — pair with `resize_dock_window_for_pill` when
/// the pill width changes, then re-measure before calling this.
#[cfg(target_os = "macos")]
pub fn sync_vibrancy_pill_from_web(
    window: &WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let position = current_dock_position(window);
    set_vibrancy_pill_frame(window, width, height, Some(x), Some(y), position)?;

    let state = window.state::<AppsState>();
    {
        let mut current_width = state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_width = width;
    }
    {
        let mut current_height = state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_height = height;
    }

    // Idempotent — deferred from `setup_dock_window` so the first paint
    // already has a DOM-aligned vibrancy mask.
    window.show().map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_dock_vibrancy(
    _window: &WebviewWindow,
    _pill_width_dip: f64,
    _pill_height_dip: f64,
    _position: DockPosition,
) -> Result<(), String> {
    Ok(())
}

/// Installs an `NSTrackingActiveAlways` area on the window content view so
/// mouse-enter/move events reach the WebView even when another app is key.
/// Does not call `makeKeyWindow` — the dock must not steal focus on hover.
#[cfg(target_os = "macos")]
fn enable_inactive_mouse_tracking(window: &WebviewWindow) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSTrackingArea, NSTrackingAreaOptions, NSWindow};

    let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())? as *mut NSWindow;
    let ns_window = unsafe { &*ns_window_ptr };

    ns_window.setAcceptsMouseMovedEvents(true);

    let content_view = ns_window
        .contentView()
        .ok_or_else(|| "no content view".to_string())?;

    let bounds = content_view.bounds();
    let options = NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::ActiveAlways
        | NSTrackingAreaOptions::InVisibleRect;

    let mtm = MainThreadMarker::new().ok_or("not on main thread")?;
    let tracking_area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            mtm.alloc::<NSTrackingArea>(),
            bounds,
            options,
            Some(content_view.as_ref()),
            None,
        )
    };

    content_view.addTrackingArea(&tracking_area);

    Ok(())
}

/// Pill hit-test rect in screen coordinates: anchored to the near edge for
/// `position` (`pill_w`/`pill_h` already include any hover-extension on
/// the thickness axis, applied by the caller), centered on the other
/// screen dimension. Mirrors `apply_dock_window_frame`'s anchoring model,
/// in physical pixels — see PROMPT_15_POSITION_PHASE1.md.
#[cfg(target_os = "macos")]
fn pill_rect_for_position(
    position: DockPosition,
    outer_pos: PhysicalPosition<i32>,
    outer_size: PhysicalSize<u32>,
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

/// Maps a screen-space cursor position to DIP coords inside the window when
/// the point lies on the rounded pill footprint; `None` otherwise. Reads
/// the pill's current *length* from `AppsState` (mutated by
/// `setup_dock_window` / `sync_vibrancy_pill_from_web`) instead of a
/// compile-time constant, so both consumers below (the click tap and the
/// hover poller) automatically hit-test against whatever size is currently
/// applied. `pill_thickness_dip` is the caller-supplied thickness (rest or
/// hover-extended, per `pill_thickness_hover_dip`) — the one dimension
/// that varies with hover state.
#[cfg(target_os = "macos")]
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

/// Like `pill_cursor_at_screen`, but in `BelowWindows` mode returns `None`
/// when another app's window is visually on top of the dock at that point.
#[cfg(target_os = "macos")]
fn pill_cursor_at_screen_if_actionable(
    window: &WebviewWindow,
    screen_x: i32,
    screen_y: i32,
    dock_hovered: bool,
) -> Option<DockCursorPayload> {
    let payload = pill_cursor_at_screen(window, screen_x, screen_y, dock_hovered)?;
    if matches!(current_dock_window_layer(window), DockWindowLayer::BelowWindows)
        && !dock_is_topmost_at_screen_point(window, screen_x, screen_y)
    {
        return None;
    }
    Some(payload)
}

/// `NSWindow::windowNumber` for the dock webview — equals `kCGWindowNumber`.
#[cfg(target_os = "macos")]
fn dock_window_number(window: &WebviewWindow) -> Option<i32> {
    use objc2_app_kit::NSWindow;

    let ns_window_ptr = window.ns_window().ok()? as *mut NSWindow;
    let ns_window = unsafe { &*ns_window_ptr };
    Some(ns_window.windowNumber() as i32)
}

#[cfg(target_os = "macos")]
fn cf_number_as_f64(value: &core_foundation::base::CFType) -> Option<f64> {
    use core_foundation::number::CFNumber;

    value.downcast::<CFNumber>()?.to_f64()
}

/// `kCGWindowBounds` uses screen points with a top-left origin on the primary
/// display — convert Tauri physical pixels via `scale` before comparing.
#[cfg(target_os = "macos")]
fn cg_window_bounds_contains_screen_point(
    bounds: &core_foundation::dictionary::CFDictionary<
        core_foundation::string::CFString,
        core_foundation::base::CFType,
    >,
    screen_x: i32,
    screen_y: i32,
    scale: f64,
) -> bool {
    use core_foundation::string::CFString;

    let x_key = CFString::new("X");
    let y_key = CFString::new("Y");
    let width_key = CFString::new("Width");
    let height_key = CFString::new("Height");

    let Some(x) = bounds.find(&x_key).as_deref().and_then(cf_number_as_f64) else {
        return false;
    };
    let Some(y) = bounds.find(&y_key).as_deref().and_then(cf_number_as_f64) else {
        return false;
    };
    let Some(width) = bounds
        .find(&width_key)
        .as_deref()
        .and_then(cf_number_as_f64)
    else {
        return false;
    };
    let Some(height) = bounds
        .find(&height_key)
        .as_deref()
        .and_then(cf_number_as_f64)
    else {
        return false;
    };

    let point_x = screen_x as f64 / scale;
    let point_y = screen_y as f64 / scale;

    point_x >= x
        && point_x <= x + width
        && point_y >= y
        && point_y <= y + height
}

/// Whether the dock is the frontmost on-screen layer-0 window at a screen
/// point. Used only in `BelowWindows` mode so clicks on covering apps are not
/// also delivered to dock icons via the HID-level event tap.
#[cfg(target_os = "macos")]
fn dock_is_topmost_at_screen_point(
    window: &WebviewWindow,
    screen_x: i32,
    screen_y: i32,
) -> bool {
    use core_foundation::base::{CFType, FromVoid, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowNumber,
    };

    let dock_number = match dock_window_number(window) {
        Some(number) => number,
        None => return true,
    };
    let scale = window.scale_factor().unwrap_or(2.0);

    let option = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let Some(windows) = copy_window_info(option, kCGNullWindowID) else {
        return true;
    };

    let number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };

    for ptr in windows.get_all_values() {
        let dict = unsafe { CFDictionary::<CFString, CFType>::from_void(ptr) };

        let Some(layer_ref) = dict.find(&layer_key) else {
            continue;
        };
        let Some(layer) = layer_ref.downcast::<CFNumber>() else {
            continue;
        };
        if layer.to_i32() != Some(0) {
            continue;
        }

        let Some(bounds_ref) = dict.find(&bounds_key) else {
            continue;
        };
        let bounds_dict = unsafe {
            CFDictionary::<CFString, CFType>::from_void(
                bounds_ref.as_concrete_TypeRef() as *const _,
            )
        };
        if !cg_window_bounds_contains_screen_point(&bounds_dict, screen_x, screen_y, scale) {
            continue;
        }

        let Some(number_ref) = dict.find(&number_key) else {
            return false;
        };
        let Some(number) = number_ref.downcast::<CFNumber>() else {
            return false;
        };
        return number.to_i32() == Some(dock_number);
    }

    true
}

/// Reads the pill's current *rest* thickness from `AppsState` (mutated by
/// `setup_dock_window`/`resize_dock_window_for_pill`/
/// `sync_vibrancy_pill_from_web`) — same "state, not compile-time constant"
/// rationale as `pill_cursor_at_screen`'s own length read, generalized
/// across positions via `DockPosition::axis`.
#[cfg(target_os = "macos")]
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

/// Converts a screen-space point to this window's logical (DIP) coordinates,
/// unconditionally — unlike `pill_cursor_at_screen`, this doesn't gate on
/// the point being inside the pill. Used for `dock-global-mousedown`, whose
/// whole point is to be delivered for clicks *outside* both the pill and
/// this window (see `start_dock_click_tap`).
#[cfg(target_os = "macos")]
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

/// HID-layer click capture — emits `dock-click` on mouse *up*, only when the
/// pointer barely moved since mouse-down. That keeps icon drag-reorder from
/// also launching the app (mousedown used to fire immediately).
#[cfg(target_os = "macos")]
fn start_dock_click_tap(window: WebviewWindow) {
    use core_foundation::mach_port::CFMachPort;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    };

    /// Squared px tolerance — same order of magnitude as a macOS dock tap.
    const CLICK_MOVE_TOLERANCE_SQ: i32 = 12 * 12;

    struct PendingDown {
        x: i32,
        y: i32,
    }

    struct LastTap {
        at: std::time::Instant,
        screen_x: i32,
        screen_y: i32,
    }

    std::thread::spawn(move || {
        let mach_port: Arc<Mutex<Option<CFMachPort>>> = Arc::new(Mutex::new(None));
        let mach_port_cb = Arc::clone(&mach_port);
        let pending_down: Arc<Mutex<Option<PendingDown>>> = Arc::new(Mutex::new(None));
        let pending_down_cb = Arc::clone(&pending_down);
        let last_tap: Arc<Mutex<Option<LastTap>>> = Arc::new(Mutex::new(None));
        let last_tap_cb = Arc::clone(&last_tap);

        let tap = match CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            vec![CGEventType::LeftMouseDown, CGEventType::LeftMouseUp],
            move |_proxy, event_type, event| {
                match event_type {
                    CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
                        if let Ok(guard) = mach_port_cb.lock() {
                            if let Some(port) = guard.as_ref() {
                                enable_event_tap(port);
                            }
                        }
                        return None;
                    }
                    CGEventType::LeftMouseDown => {
                        if let Ok(cursor) = window.cursor_position() {
                            let down_x = cursor.x.round() as i32;
                            let down_y = cursor.y.round() as i32;
                            if let Ok(mut guard) = pending_down_cb.lock() {
                                *guard = Some(PendingDown {
                                    x: down_x,
                                    y: down_y,
                                });
                            }

                            // Delivered for *every* left-mouse-down anywhere
                            // on screen, regardless of `set_ignore_cursor_events`
                            // — the frontend uses this to dismiss an open
                            // context menu on a real "click anywhere else",
                            // including clicks on other apps or the desktop,
                            // which never reach the WebView's own DOM events
                            // at all under this window's click-through design.
                            if let Some(payload) = cursor_to_window_logical(&window, down_x, down_y)
                            {
                                let _ = window.emit("dock-global-mousedown", payload);
                            }
                        }
                    }
                    CGEventType::LeftMouseUp => {
                        let is_tap = {
                            let Ok(cursor) = window.cursor_position() else {
                                if let Ok(mut guard) = pending_down_cb.lock() {
                                    *guard = None;
                                }
                                return Some(event.clone());
                            };
                            let up_x = cursor.x.round() as i32;
                            let up_y = cursor.y.round() as i32;
                            let Ok(mut guard) = pending_down_cb.lock() else {
                                return Some(event.clone());
                            };
                            let Some(down) = guard.take() else {
                                return Some(event.clone());
                            };
                            let dx = up_x - down.x;
                            let dy = up_y - down.y;
                            dx * dx + dy * dy <= CLICK_MOVE_TOLERANCE_SQ
                        };

                        if is_tap {
                            if let Ok(cursor) = window.cursor_position() {
                                let cursor_x = cursor.x.round() as i32;
                                let cursor_y = cursor.y.round() as i32;
                                if let Some(payload) = pill_cursor_at_screen_if_actionable(
                                    &window,
                                    cursor_x,
                                    cursor_y,
                                    true,
                                ) {
                                    let scale = window.scale_factor().unwrap_or(2.0);
                                    let icon_tol = (current_icon_size_dip(&window) * scale * 1.5)
                                        .round()
                                        .max(24.0) as i32;
                                    let is_double = match last_tap_cb.lock() {
                                        Ok(mut guard) => {
                                            let now = std::time::Instant::now();
                                            let double = guard.as_ref().is_some_and(|prev| {
                                                now.duration_since(prev.at)
                                                    < std::time::Duration::from_millis(
                                                        DOUBLE_CLICK_INTERVAL_MS,
                                                    )
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
                                    let _ = window.emit("dock-click", payload);
                                    if is_double {
                                        let _ = window.emit("dock-double-click", payload);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                Some(event.clone())
            },
        ) {
            Ok(tap) => tap,
            Err(()) => {
                eprintln!(
                    "GameDockPanel: failed to create CGEventTap for dock-click — \
                     grant Accessibility or Input Monitoring in System Settings"
                );
                return;
            }
        };

        if let Ok(mut guard) = mach_port.lock() {
            *guard = Some(tap.mach_port.clone());
        }

        unsafe {
            let loop_source = match tap.mach_port.create_runloop_source(0) {
                Ok(source) => source,
                Err(()) => {
                    eprintln!("GameDockPanel: failed to create run loop source for dock-click tap");
                    return;
                }
            };
            CFRunLoop::get_current().add_source(&loop_source, kCFRunLoopCommonModes);
            tap.enable();
            CFRunLoop::run_current();
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn start_dock_click_tap(_window: WebviewWindow) {}

#[cfg(target_os = "macos")]
fn enable_event_tap(port: &core_foundation::mach_port::CFMachPort) {
    use core_foundation::base::TCFType;

    extern "C" {
        fn CGEventTapEnable(tap: core_foundation::mach_port::CFMachPortRef, enable: bool);
    }
    unsafe {
        CGEventTapEnable(port.as_concrete_TypeRef(), true);
    }
}

/// Polls the global cursor and toggles `set_ignore_cursor_events` so only the
/// dock pill captures input — transparent bands above it stay click-through
/// at the OS level (WKWebView `pointer-events-none` is not sufficient alone).
#[cfg(target_os = "macos")]
fn start_click_through_poller(window: WebviewWindow) {
    std::thread::spawn(move || {
        let mut ignoring = true;
        let mut dock_hovered = false;
        let mut last_cursor: Option<DockCursorPayload> = None;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(CLICK_POLL_MS));

            let Ok(cursor) = window.cursor_position() else {
                continue;
            };
            let cursor_x = cursor.x.round() as i32;
            let cursor_y = cursor.y.round() as i32;

            let pill_cursor =
                pill_cursor_at_screen_if_actionable(&window, cursor_x, cursor_y, dock_hovered);
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

#[cfg(not(target_os = "macos"))]
fn start_click_through_poller(_window: WebviewWindow) {}

/// Point-in-rounded-rect test — `radius` cuts the same 4 corners the CSS
/// `rounded-[28px]` pill draws, so click-through matches the visible shape
/// instead of its (larger) bounding box.
#[cfg(target_os = "macos")]
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

// --- Real process monitoring (PROMPT_04_PROCESS_MONITORING.md) ---
//
// Event-driven, not polled: one `runningApplications()` snapshot at startup,
// then only incremental updates from `NSWorkspace`'s launch/terminate
// notifications. See the `tauri-glass-dock` skill for the thread-safety
// rationale behind keeping `AppsState` as plain data instead of live
// `NSRunningApplication` handles.
//
// Generalized in PROMPT_06_CUSTOM_APPS.md: both the startup snapshot and the
// notification handler below read `AppsState.entries` fresh at the moment
// they run, rather than closing over a fixed roster — the observer blocks
// are registered once (`mem::forget`) and live for the whole process, but
// `entries` keeps changing underneath them as the user adds/removes apps.

/// Snapshots current state, subscribes to launch/terminate notifications,
/// and resolves+caches icons for the dock roster. Runs on the main
/// thread (guaranteed by Tauri's `.setup()`), same as `setup_dock_window`.
#[cfg(target_os = "macos")]
pub fn start_apps_monitoring(app: &App) -> Result<(), String> {
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidLaunchApplicationNotification,
        NSWorkspaceDidTerminateApplicationNotification,
    };

    let app_handle = app.handle().clone();
    let state = app.state::<AppsState>();
    let workspace = NSWorkspace::sharedWorkspace();

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for item in entries.iter() {
            let DockItem::App(entry) = item else {
                continue;
            };
            running.insert(
                entry.bundle_id.clone(),
                live_bundle_running(&entry.bundle_id),
            );
        }
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let icon_size_dip = current_icon_size_dip_from_app(&app_handle);
        let scale_factor = dock_window_scale_factor(&app_handle);
        for item in entries.iter() {
            let DockItem::App(entry) = item else {
                continue;
            };
            let resolved = resolve_app_icon(&entry.bundle_id, icon_size_dip, scale_factor);
            crate::commands::apps::apply_icon_resolve(&state, &entry.bundle_id, resolved);
        }
    }

    let notification_center = workspace.notificationCenter();

    let launch_handle = app_handle.clone();
    let launch_token = unsafe {
        notification_center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidLaunchApplicationNotification),
            None,
            None,
            &block2::RcBlock::new(move |notification| {
                handle_workspace_notification(&launch_handle, notification, true);
            }),
        )
    };
    // Intentional permanent leak: these observers must outlive this function
    // and live for the whole process — dropping the token would let ARC
    // deallocate it and silently stop delivery. There is no natural point
    // in this app's lifecycle to ever unregister them.
    std::mem::forget(launch_token);

    let terminate_handle = app_handle.clone();
    let terminate_token = unsafe {
        notification_center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidTerminateApplicationNotification),
            None,
            None,
            &block2::RcBlock::new(move |notification| {
                handle_workspace_notification(&terminate_handle, notification, false);
            }),
        )
    };
    std::mem::forget(terminate_token);

    start_running_reconcile_poller(app_handle.clone());

    emit_apps_icons_updated(&app_handle, app.state::<AppsState>().icons_snapshot());
    emit_apps_running_changed(&app_handle);
    crate::commands::apps::emit_apps_list_changed(&app_handle, &app.state::<AppsState>());

    Ok(())
}

#[cfg(target_os = "macos")]
fn handle_workspace_notification(
    app: &AppHandle,
    notification: std::ptr::NonNull<objc2_foundation::NSNotification>,
    is_launch: bool,
) {
    use objc2_app_kit::{NSRunningApplication, NSWorkspaceApplicationKey};

    let notification = unsafe { notification.as_ref() };
    let Some(user_info) = notification.userInfo() else {
        return;
    };
    let Some(running_app_obj) =
        (unsafe { user_info.objectForKey(NSWorkspaceApplicationKey) })
    else {
        return;
    };
    let Some(running_app) = running_app_obj.downcast_ref::<NSRunningApplication>() else {
        return;
    };
    let Some(bundle_id) = running_app.bundleIdentifier() else {
        return;
    };
    let bundle_id = bundle_id.to_string();

    let state = app.state::<AppsState>();

    let is_tracked = {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.iter().any(|item| {
            matches!(
                item,
                DockItem::App(entry) if entry.bundle_id == bundle_id
            )
        })
    };
    if !is_tracked {
        // Not one of our tracked apps — ignore.
        return;
    }

    {
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        running.insert(bundle_id.clone(), is_launch);
    }

    // Icons are normally resolved once at startup / at add-time (see
    // `start_apps_monitoring` / `commands::apps::add_app_from_path`).
    // If a tracked app's icon somehow never resolved, retry only on the
    // (rare) launch path, and only if still unresolved — not a hot path,
    // no new polling/infrastructure.
    let mut icon_resolved = false;
    if is_launch {
        let needs_resolve = {
            let icons = state
                .icons
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            icons.get(&bundle_id).cloned().flatten().is_none()
        };
        if needs_resolve {
            let icon_size_dip = current_icon_size_dip_from_app(app);
            let scale_factor = dock_window_scale_factor(app);
            let resolved = resolve_app_icon(&bundle_id, icon_size_dip, scale_factor);
            if resolved.icon_url.is_some() {
                crate::commands::apps::apply_icon_resolve(&state, &bundle_id, resolved);
                icon_resolved = true;
            }
        }
    }

    emit_apps_running_changed(app);
    if icon_resolved {
        if let Some(update) = state.icon_update_for(&bundle_id) {
            emit_apps_icons_updated(app, vec![update]);
        }
    }
}

#[cfg(target_os = "macos")]
fn emit_apps_running_changed(app: &AppHandle) {
    let state = app.state::<AppsState>();
    let payload = state.running_snapshot();
    let _ = app.emit("apps-running-changed", payload);
}

/// Whether `bundle_id` currently has a live (non-terminated) process.
#[cfg(target_os = "macos")]
fn live_bundle_running(bundle_id: &str) -> bool {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    let ns_bundle_id = NSString::from_str(bundle_id);
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&ns_bundle_id)
        .iter()
        .any(|instance| !instance.isTerminated())
}

/// Re-reads the live process list for one bundle ID, updates `AppsState.running`
/// when it diverges, and emits `apps-running-changed`. Returns the live value.
#[cfg(target_os = "macos")]
fn sync_bundle_running_state(app: &AppHandle, bundle_id: &str) -> bool {
    let live = live_bundle_running(bundle_id);
    let state = app.state::<AppsState>();
    let changed = {
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored = running.get(bundle_id).copied().unwrap_or(false);
        if stored != live {
            running.insert(bundle_id.to_string(), live);
            true
        } else {
            false
        }
    };
    if changed {
        emit_apps_running_changed(app);
    }
    live
}

/// After a cold `open_path`, `didLaunch` can be late or missing — poll the
/// live process list on the main thread until the bundle is running or the
/// retry schedule is exhausted.
#[cfg(target_os = "macos")]
fn start_launch_running_watch(app: AppHandle, bundle_id: String) {
    std::thread::spawn(move || {
        for &delay_ms in LAUNCH_WATCH_DELAYS_MS {
            if delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }

            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let app_for_main = app.clone();
            let bundle_id_for_main = bundle_id.clone();
            if app
                .run_on_main_thread(move || {
                    let running =
                        sync_bundle_running_state(&app_for_main, &bundle_id_for_main);
                    let _ = tx.send(running);
                })
                .is_err()
            {
                break;
            }

            if rx.recv().unwrap_or(false) {
                break;
            }
        }
    });
}

/// Backup for missed `didLaunch` / `didTerminate` — reconciles every tracked
/// bundle ID against the live process list on a slow interval separate from
/// the 50 ms click-through poller.
#[cfg(target_os = "macos")]
fn start_running_reconcile_poller(app_handle: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(
                RUNNING_RECONCILE_POLL_MS,
            ));

            let has_tracked_apps = {
                let state = app_handle.state::<AppsState>();
                let entries = state
                    .entries
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                entries
                    .iter()
                    .any(|item| matches!(item, DockItem::App(_)))
            };
            if !has_tracked_apps {
                continue;
            }

            let app = app_handle.clone();
            let app_for_main = app.clone();
            let _ = app.run_on_main_thread(move || {
                reconcile_running_apps_on_main_thread(&app_for_main);
            });
        }
    });
}

#[cfg(target_os = "macos")]
fn reconcile_running_apps_on_main_thread(app: &AppHandle) {
    let state = app.state::<AppsState>();

    let candidates: Vec<String> = {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries
            .iter()
            .filter_map(|item| {
                let DockItem::App(entry) = item else {
                    return None;
                };
                Some(entry.bundle_id.clone())
            })
            .collect()
    };

    if candidates.is_empty() {
        return;
    }

    let mut changed = false;
    for bundle_id in candidates {
        let live = live_bundle_running(&bundle_id);
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored = running.get(&bundle_id).copied().unwrap_or(false);
        if stored != live {
            running.insert(bundle_id, live);
            changed = true;
        }
    }

    if changed {
        emit_apps_running_changed(app);
    }
}

#[cfg(target_os = "macos")]
fn emit_apps_icons_updated(app: &AppHandle, updates: Vec<crate::commands::apps::AppIconUpdatePayload>) {
    let _ = app.emit("apps-icons-updated", updates);
}

/// Re-rasterizes every dock icon at the current display metrics — called
/// when `iconSizePx` changes so cached PNGs stay sharp on Retina.
#[cfg(target_os = "macos")]
pub fn refresh_dock_icons(app: &AppHandle, state: &AppsState) {
    let icon_size_dip = current_icon_size_dip_from_app(app);
    let scale_factor = dock_window_scale_factor(app);
    // Cloned snapshot, NOT a held `entries` guard: `icons_snapshot()` below
    // re-locks `entries` on this same thread, so keeping the guard alive
    // across the loop self-deadlocked the main thread (std::sync::Mutex is
    // not reentrant). Masked for a while by the preview-write bug in
    // `sync_icon_size_preview` that made `apply_dock_settings` skip this
    // refresh — found live in the PROMPT_17 QA pass once that was fixed.
    let entries = state.entries_snapshot();

    for item in entries.iter() {
        let DockItem::App(entry) = item else {
            continue;
        };
        let resolved = resolve_app_icon(&entry.bundle_id, icon_size_dip, scale_factor);
        crate::commands::apps::apply_icon_resolve(state, &entry.bundle_id, resolved);
    }

    emit_apps_icons_updated(app, state.icons_snapshot());
    crate::commands::apps::emit_apps_list_changed(app, state);
}

/// Resolves a bundle ID to its installed `.app`'s icon and renders it as a
/// `data:image/png;base64,...` URL, sampling an accent color from the same
/// bitmap. Returns empty fields if the app isn't installed or encoding fails.
#[cfg(target_os = "macos")]
pub fn resolve_app_icon(
    bundle_id: &str,
    icon_size_dip: f64,
    scale_factor: f64,
) -> IconResolveResult {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    let workspace = NSWorkspace::sharedWorkspace();
    let ns_bundle_id = NSString::from_str(bundle_id);
    let Some(app_url) = workspace.URLForApplicationWithBundleIdentifier(&ns_bundle_id) else {
        return IconResolveResult::default();
    };
    let Some(path) = app_url.path() else {
        return IconResolveResult::default();
    };
    let icon = workspace.iconForFile(&path);
    let export_px = icon_export_px(icon_size_dip, scale_factor);
    icon_to_png_and_accent(&icon, export_px)
}

#[cfg(target_os = "macos")]
fn current_icon_size_dip_from_app(app: &AppHandle) -> f64 {
    if let Some(window) = app.get_webview_window("main") {
        return current_icon_size_dip(&window);
    }

    let state = app.state::<SettingsState>();
    let guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.icon_size_px
}

#[cfg(target_os = "macos")]
fn dock_window_scale_factor(app: &AppHandle) -> f64 {
    app.get_webview_window("main")
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(2.0)
}

/// `NSImage` → PNG bytes via `CGImage`, not manual `.icns`/`Info.plist`
/// parsing (see PROMPT_04_PROCESS_MONITORING.md point 6). `NSImage` doesn't
/// expose a modern direct-to-PNG method, so the standard AppKit route is
/// `NSImage` → `CGImage` → `NSBitmapImageRep` → PNG `NSData`.
#[cfg(target_os = "macos")]
fn icon_to_png_and_accent(icon: &objc2_app_kit::NSImage, export_px: f64) -> IconResolveResult {
    use base64::Engine as _;
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

    let properties = NSDictionary::new();
    let min_pixels = (export_px * 0.75).floor() as i64;
    let mut best_png: Option<Vec<u8>> = None;
    let mut best_pixels: i64 = 0;
    let mut best_accent: Option<String> = None;

    for rep in icon.representations().iter() {
        let Some(bitmap) = rep.downcast_ref::<NSBitmapImageRep>() else {
            continue;
        };
        let px = bitmap.pixelsWide().max(bitmap.pixelsHigh()) as i64;
        if px <= best_pixels {
            continue;
        }
        let accent = extract_accent_color_from_bitmap(bitmap);
        if let Some(png_data) = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        } {
            best_pixels = px;
            best_png = Some(png_data.to_vec());
            best_accent = accent;
        }
    }

    if let Some(png_bytes) = best_png {
        if best_pixels >= min_pixels {
            return IconResolveResult {
                icon_url: Some(format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(png_bytes)
                )),
                accent_color: best_accent,
            };
        }
    }

    // Fallback: rasterize at the target export size via CGImage.
    let export_size = NSSize::new(export_px, export_px);
    icon.setSize(export_size);
    let mut proposed_rect = NSRect::new(NSPoint::new(0.0, 0.0), export_size);
    let cg_image = unsafe {
        icon.CGImageForProposedRect_context_hints(&mut proposed_rect, None, None)
    };
    let Some(cg_image) = cg_image else {
        return IconResolveResult::default();
    };

    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg_image);
    let accent = extract_accent_color_from_bitmap(&bitmap);
    let png_data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    };
    let Some(png_data) = png_data else {
        return IconResolveResult::default();
    };

    IconResolveResult {
        icon_url: Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png_data.to_vec())
        )),
        accent_color: accent,
    }
}

/// Picks a saturated accent color from icon pixels for the running-app LED.
#[cfg(target_os = "macos")]
fn extract_accent_color_from_bitmap(bitmap: &objc2_app_kit::NSBitmapImageRep) -> Option<String> {
    let width = bitmap.pixelsWide();
    let height = bitmap.pixelsHigh();
    if width <= 0 || height <= 0 {
        return None;
    }

    let bytes_per_row = bitmap.bytesPerRow();
    let samples_per_pixel = bitmap.samplesPerPixel();
    if bytes_per_row <= 0 || samples_per_pixel < 3 {
        return None;
    }

    let data = bitmap.bitmapData();
    if data.is_null() {
        return None;
    }

    const HUE_BUCKETS: usize = 16;
    let mut bucket_weight = [0.0f64; HUE_BUCKETS];
    let mut bucket_hue = [0.0f64; HUE_BUCKETS];
    let mut bucket_sat = [0.0f64; HUE_BUCKETS];
    let mut bucket_light = [0.0f64; HUE_BUCKETS];
    let mut bucket_count = [0u32; HUE_BUCKETS];

    let step_x = (width / 32).max(1);
    let step_y = (height / 32).max(1);

    for y in (0..height).step_by(step_y as usize) {
        for x in (0..width).step_by(step_x as usize) {
            let Some((r, g, b, a)) = read_bitmap_rgba(
                data,
                bytes_per_row,
                samples_per_pixel,
                x,
                y,
            ) else {
                continue;
            };
            if a < 128 {
                continue;
            }

            let (h, s, l) = rgb_to_hsl(r, g, b);
            if l < 0.15 || l > 0.92 || s < 0.25 {
                continue;
            }

            let bucket = ((h * HUE_BUCKETS as f64).floor() as usize).min(HUE_BUCKETS - 1);
            let weight = s * s;
            bucket_weight[bucket] += weight;
            bucket_hue[bucket] += h * weight;
            bucket_sat[bucket] += s * weight;
            bucket_light[bucket] += l * weight;
            bucket_count[bucket] += 1;
        }
    }

    let (best_bucket, _) = bucket_weight
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;

    if bucket_weight[best_bucket] <= 0.0 || bucket_count[best_bucket] == 0 {
        return None;
    }

    let total = bucket_weight[best_bucket];
    let h = bucket_hue[best_bucket] / total;
    let s = (bucket_sat[best_bucket] / total * 1.1).clamp(0.35, 1.0);
    let l = (bucket_light[best_bucket] / total).clamp(0.5, 0.72);
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
}

#[cfg(target_os = "macos")]
fn read_bitmap_rgba(
    data: *mut u8,
    bytes_per_row: objc2_foundation::NSInteger,
    samples_per_pixel: objc2_foundation::NSInteger,
    x: objc2_foundation::NSInteger,
    y: objc2_foundation::NSInteger,
) -> Option<(u8, u8, u8, u8)> {
    let spp = samples_per_pixel as isize;
    let offset = (y as isize) * bytes_per_row as isize + (x as isize) * spp;
    if offset < 0 {
        return None;
    }
    unsafe {
        let base = data.offset(offset);
        let r = *base;
        let g = *base.add(1);
        let b = *base.add(2);
        let a = if spp > 3 { *base.add(3) } else { 255 };
        Some((r, g, b, a))
    }
}

#[cfg(target_os = "macos")]
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f64::EPSILON {
        let mut hue = (g - b) / d;
        if g < b {
            hue += 6.0;
        }
        hue / 6.0
    } else if (max - g).abs() < f64::EPSILON {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

#[cfg(target_os = "macos")]
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    if s <= f64::EPSILON {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let r = hue_to_rgb_channel(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb_channel(p, q, h);
    let b = hue_to_rgb_channel(p, q, h - 1.0 / 3.0);
    (
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

#[cfg(target_os = "macos")]
fn hue_to_rgb_channel(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 1.0 / 2.0 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// Whether a bundle ID resolves to an installed `.app` at all — used to
/// filter `SEED_POOL` candidates on first run (`commands::apps::seed_entries`).
/// Same underlying `NSWorkspace` lookup as icon/launch resolution, just
/// checking presence instead of using the result.
#[cfg(target_os = "macos")]
pub fn is_app_installed(bundle_id: &str) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    let workspace = NSWorkspace::sharedWorkspace();
    let ns_bundle_id = NSString::from_str(bundle_id);
    workspace
        .URLForApplicationWithBundleIdentifier(&ns_bundle_id)
        .is_some()
}

/// Whether a bundle ID currently has a running instance — used when adding
/// an app at runtime (`commands::apps::add_app_from_path`) to seed its
/// initial LED state without waiting for the next `NSWorkspace` event.
#[cfg(target_os = "macos")]
pub fn is_bundle_running(bundle_id: &str) -> bool {
    live_bundle_running(bundle_id)
}

/// Resolves the `CFBundleIdentifier` of a `.app` bundle at `path` — the
/// reverse of `resolve_icon_data_url`'s bundle-id-to-path lookup, needed for
/// the Finder drag-drop flow where all we have is a filesystem path the
/// user dropped.
#[cfg(target_os = "macos")]
pub fn resolve_bundle_id_from_path(path: &str) -> Result<String, String> {
    use objc2_foundation::{NSBundle, NSString};

    let ns_path = NSString::from_str(path);
    let bundle = NSBundle::bundleWithPath(&ns_path)
        .ok_or_else(|| format!("{path} is not a valid app bundle"))?;
    let bundle_id = bundle
        .bundleIdentifier()
        .ok_or_else(|| format!("{path} has no CFBundleIdentifier"))?;
    Ok(bundle_id.to_string())
}

// ── Dock-aware window zoom (double-click on running app) ───────────────────

/// Screen-space rectangle in Tauri physical pixels (global top-left origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScreenRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ScreenRect {
    fn right(&self) -> i32 {
        self.x + self.width
    }

    fn bottom(&self) -> i32 {
        self.y + self.height
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGSize {
    width: f64,
    height: f64,
}

type AxUiElementRef = *mut std::ffi::c_void;
type CfTypeRef = *const std::ffi::c_void;

const K_AX_ERROR_SUCCESS: i32 = 0;
const K_AX_VALUE_CGPOINT_TYPE: u32 = 1;
const K_AX_VALUE_CGSIZE_TYPE: u32 = 2;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AxUiElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AxUiElementRef,
        attribute: CfTypeRef,
        value: *mut CfTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AxUiElementRef,
        attribute: CfTypeRef,
        value: CfTypeRef,
    ) -> i32;
    fn AXIsProcessTrustedWithOptions(options: CfTypeRef) -> bool;
    fn AXValueCreate(value_type: u32, value_ptr: *const std::ffi::c_void) -> CfTypeRef;
    fn AXValueGetValue(value: CfTypeRef, value_type: u32, value_ptr: *mut std::ffi::c_void) -> bool;
    fn CFRelease(cf: CfTypeRef);
}

#[cfg(target_os = "macos")]
fn primary_cocoa_height_points(mtm: objc2::MainThreadMarker) -> f64 {
    use objc2_app_kit::NSScreen;

    NSScreen::mainScreen(mtm)
        .map(|screen| screen.frame().size.height)
        .unwrap_or(0.0)
}

#[cfg(target_os = "macos")]
fn cfstring_attr(name: &str) -> core_foundation::string::CFString {
    core_foundation::string::CFString::new(name)
}

#[cfg(target_os = "macos")]
fn ensure_ax_trusted() -> Result<(), String> {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;

    let key = cfstring_attr("AXTrustedCheckOptionPrompt");
    let prompt = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key, prompt)]);
    let trusted =
        unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef() as CfTypeRef) };
    if trusted {
        Ok(())
    } else {
        Err(
            "enable Accessibility for GameDockPanel in System Settings → Privacy & Security → Accessibility"
                .to_string(),
        )
    }
}

#[cfg(target_os = "macos")]
fn nsscreen_visible_tauri(screen: &objc2_app_kit::NSScreen, primary_cocoa_h: f64) -> ScreenRect {
    let frame = screen.frame();
    let visible = screen.visibleFrame();
    let backing = screen.backingScaleFactor();

    let x = (visible.origin.x * backing).round() as i32;
    let width = (visible.size.width * backing).round() as i32;
    let height = (visible.size.height * backing).round() as i32;
    let y = (primary_cocoa_h * backing
        - visible.origin.y * backing
        - visible.size.height * backing)
        .round() as i32;

    // Silence unused `frame` when visible spans the full screen height.
    let _ = frame;

    ScreenRect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(target_os = "macos")]
fn visible_frame_tauri_for_monitor(
    monitor: &tauri::Monitor,
    mtm: objc2::MainThreadMarker,
) -> ScreenRect {
    use objc2_app_kit::NSScreen;

    let primary_cocoa_h = primary_cocoa_height_points(mtm);
    let mon_size = monitor.size();
    let mon_pos = monitor.position();
    let scale = monitor.scale_factor();

    for screen in NSScreen::screens(mtm).iter() {
        let frame = screen.frame();
        let backing = screen.backingScaleFactor();
        let screen_w = (frame.size.width * backing).round() as u32;
        let screen_h = (frame.size.height * backing).round() as u32;
        let screen_x = (frame.origin.x * backing).round() as i32;
        let screen_y = (primary_cocoa_h * backing
            - frame.origin.y * backing
            - frame.size.height * backing)
            .round() as i32;

        if screen_w == mon_size.width
            && screen_h == mon_size.height
            && screen_x == mon_pos.x
            && (screen_y - mon_pos.y).abs() <= 2
        {
            return nsscreen_visible_tauri(&screen, primary_cocoa_h);
        }
    }

    // Fallback when no NSScreen match — menu bar only, no system-dock guess.
    let menu_bar = (25.0 * scale).round() as i32;
    ScreenRect {
        x: mon_pos.x,
        y: mon_pos.y + menu_bar,
        width: mon_size.width as i32,
        height: mon_size.height as i32 - menu_bar,
    }
}

/// Shrinks the NSScreen `visibleFrame` (menu bar + system dock already
/// excluded) so the near edge stops above/outside the GameDockPanel pill.
#[cfg(target_os = "macos")]
fn shrink_visible_for_dock(
    visible: ScreenRect,
    pill_left: i32,
    pill_top: i32,
    pill_right: i32,
    pill_bottom: i32,
    position: DockPosition,
    gap: i32,
) -> Option<ScreenRect> {
    match position {
        DockPosition::Bottom => {
            let bottom = pill_top.saturating_sub(gap);
            let height = bottom.saturating_sub(visible.y);
            if height < 32 {
                return None;
            }
            Some(ScreenRect {
                x: visible.x,
                y: visible.y,
                width: visible.width,
                height,
            })
        }
        DockPosition::Top => {
            let top = pill_bottom.saturating_add(gap);
            let bottom = visible.bottom();
            let height = bottom.saturating_sub(top);
            if height < 32 {
                return None;
            }
            Some(ScreenRect {
                x: visible.x,
                y: top,
                width: visible.width,
                height,
            })
        }
        DockPosition::Left => {
            let left = pill_right.saturating_add(gap);
            let right = visible.right();
            let width = right.saturating_sub(left);
            if width < 32 {
                return None;
            }
            Some(ScreenRect {
                x: left,
                y: visible.y,
                width,
                height: visible.height,
            })
        }
        DockPosition::Right => {
            let right = pill_left.saturating_sub(gap);
            let width = right.saturating_sub(visible.x);
            if width < 32 {
                return None;
            }
            Some(ScreenRect {
                x: visible.x,
                y: visible.y,
                width,
                height: visible.height,
            })
        }
    }
}

#[cfg(target_os = "macos")]
fn current_pill_screen_rect(window: &WebviewWindow) -> Result<(i32, i32, i32, i32), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let outer_pos = window.outer_position().map_err(|e| e.to_string())?;
    let outer_size = window.outer_size().map_err(|e| e.to_string())?;
    let position = current_dock_position(window);
    let pill_thickness = current_pill_thickness_rest_dip(window);
    let state = window.state::<AppsState>();
    let stored_width = *state
        .pill_width_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let stored_height = *state
        .pill_height_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (pill_length_dip, _) = axis_css_dims(position.axis(), stored_width, stored_height);
    let (pill_w_dip, pill_h_dip) =
        axis_css_dims(position.axis(), pill_length_dip, pill_thickness);
    let pill_w = (pill_w_dip * scale).round() as i32;
    let pill_h = (pill_h_dip * scale).round() as i32;
    let inset = (DOCK_EDGE_INSET_DIP * scale).round() as i32;
    Ok(pill_rect_for_position(
        position,
        outer_pos,
        outer_size,
        pill_w,
        pill_h,
        inset,
    ))
}

#[cfg(target_os = "macos")]
fn usable_screen_rect_for_zoom(
    window: &WebviewWindow,
    mtm: objc2::MainThreadMarker,
) -> Result<ScreenRect, String> {
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no primary monitor".to_string())?;
    let visible = visible_frame_tauri_for_monitor(&monitor, mtm);
    let (pill_left, pill_top, pill_right, pill_bottom) = current_pill_screen_rect(window)?;
    let position = current_dock_position(window);
    shrink_visible_for_dock(
        visible,
        pill_left,
        pill_top,
        pill_right,
        pill_bottom,
        position,
        ZOOM_DOCK_GAP_PX,
    )
    .ok_or_else(|| "usable zoom area is too small".to_string())
}

#[cfg(target_os = "macos")]
fn rects_approximately_equal(a: ScreenRect, b: ScreenRect, tolerance: i32) -> bool {
    (a.x - b.x).abs() <= tolerance
        && (a.y - b.y).abs() <= tolerance
        && (a.width - b.width).abs() <= tolerance
        && (a.height - b.height).abs() <= tolerance
}

#[cfg(target_os = "macos")]
fn ax_copy_attr(element: AxUiElementRef, attr: &str) -> Option<CfTypeRef> {
    use core_foundation::base::TCFType;

    let attr_cf = cfstring_attr(attr);
    let mut value: CfTypeRef = std::ptr::null();
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            attr_cf.as_concrete_TypeRef() as CfTypeRef,
            &mut value,
        )
    };
    if status == K_AX_ERROR_SUCCESS && !value.is_null() {
        Some(value)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn ax_cfstring_value(value: CfTypeRef) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    if value.is_null() {
        return None;
    }
    let cf = unsafe { CFString::wrap_under_create_rule(value as _) };
    Some(cf.to_string())
}

#[cfg(target_os = "macos")]
fn ax_window_storage_key(window_el: AxUiElementRef, bundle_id: &str) -> String {
    if let Some(id_ref) = ax_copy_attr(window_el, "AXIdentifier") {
        let id = ax_cfstring_value(id_ref);
        unsafe {
            CFRelease(id_ref);
        }
        if let Some(id) = id.filter(|value| !value.is_empty()) {
            return format!("{bundle_id}:{id}");
        }
    }
    if let Some(title_ref) = ax_copy_attr(window_el, "AXTitle") {
        let title = ax_cfstring_value(title_ref);
        unsafe {
            CFRelease(title_ref);
        }
        if let Some(title) = title.filter(|value| !value.is_empty()) {
            return format!("{bundle_id}:{title}");
        }
    }
    format!("{bundle_id}:front")
}

#[cfg(target_os = "macos")]
fn ax_window_frame_tauri(
    window_el: AxUiElementRef,
    primary_cocoa_h: f64,
    scale: f64,
) -> Result<ScreenRect, String> {
    let pos_ref = ax_copy_attr(window_el, "AXPosition").ok_or("window has no AXPosition")?;
    let size_ref = ax_copy_attr(window_el, "AXSize").ok_or("window has no AXSize")?;

    let mut pos = CGPoint { x: 0.0, y: 0.0 };
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok_pos = unsafe {
        AXValueGetValue(
            pos_ref,
            K_AX_VALUE_CGPOINT_TYPE,
            &mut pos as *mut _ as *mut std::ffi::c_void,
        )
    };
    let ok_size = unsafe {
        AXValueGetValue(
            size_ref,
            K_AX_VALUE_CGSIZE_TYPE,
            &mut size as *mut _ as *mut std::ffi::c_void,
        )
    };
    unsafe {
        CFRelease(pos_ref);
        CFRelease(size_ref);
    }
    if !ok_pos || !ok_size {
        return Err("failed to read window frame from Accessibility".to_string());
    }

    let width = (size.width * scale).round() as i32;
    let height = (size.height * scale).round() as i32;
    let x = (pos.x * scale).round() as i32;
    let y = (primary_cocoa_h * scale - pos.y * scale - height as f64).round() as i32;
    Ok(ScreenRect {
        x,
        y,
        width,
        height,
    })
}

#[cfg(target_os = "macos")]
fn set_ax_window_frame_tauri(
    window_el: AxUiElementRef,
    rect: ScreenRect,
    primary_cocoa_h: f64,
    scale: f64,
) -> Result<(), String> {
    use core_foundation::base::TCFType;

    let w_pts = rect.width as f64 / scale;
    let h_pts = rect.height as f64 / scale;
    let x_pts = rect.x as f64 / scale;
    let y_top_pts = rect.y as f64 / scale;
    let ax_y_pts = primary_cocoa_h - y_top_pts - h_pts;

    let pos = CGPoint {
        x: x_pts,
        y: ax_y_pts,
    };
    let size = CGSize {
        width: w_pts,
        height: h_pts,
    };

    let pos_val = unsafe { AXValueCreate(K_AX_VALUE_CGPOINT_TYPE, &pos as *const _ as *const _) };
    let size_val = unsafe { AXValueCreate(K_AX_VALUE_CGSIZE_TYPE, &size as *const _ as *const _) };
    if pos_val.is_null() || size_val.is_null() {
        return Err("failed to create AXValue for window frame".to_string());
    }

    let pos_attr = cfstring_attr("AXPosition");
    let size_attr = cfstring_attr("AXSize");
    let status_pos = unsafe {
        AXUIElementSetAttributeValue(
            window_el,
            pos_attr.as_concrete_TypeRef() as CfTypeRef,
            pos_val,
        )
    };
    let status_size = unsafe {
        AXUIElementSetAttributeValue(
            window_el,
            size_attr.as_concrete_TypeRef() as CfTypeRef,
            size_val,
        )
    };
    unsafe {
        CFRelease(pos_val);
        CFRelease(size_val);
    }

    if status_pos != K_AX_ERROR_SUCCESS || status_size != K_AX_ERROR_SUCCESS {
        return Err(
            "failed to resize window via Accessibility — the app may be fullscreen or block AX"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn ax_focused_window(app_el: AxUiElementRef) -> Option<AxUiElementRef> {
    if let Some(win) = ax_copy_attr(app_el, "AXFocusedWindow") {
        return Some(win as AxUiElementRef);
    }
    if let Some(win) = ax_copy_attr(app_el, "AXMainWindow") {
        return Some(win as AxUiElementRef);
    }
    None
}

#[cfg(target_os = "macos")]
fn zoom_app_above_dock_on_main_thread(
    dock_window: &WebviewWindow,
    zoom_state: &crate::commands::apps::ZoomState,
    bundle_id: &str,
) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    let mtm = MainThreadMarker::new().ok_or("not on main thread")?;
    ensure_ax_trusted()?;

    let ns_bundle_id = NSString::from_str(bundle_id);
    let running = NSRunningApplication::runningApplicationsWithBundleIdentifier(&ns_bundle_id);
    let instance = running
        .iter()
        .next()
        .ok_or_else(|| format!("{bundle_id} is not running"))?;

    activate_running_application(&instance)?;

    let pid = instance.processIdentifier();
    let scale = dock_window.scale_factor().map_err(|e| e.to_string())?;
    let primary_cocoa_h = primary_cocoa_height_points(mtm);
    let usable = usable_screen_rect_for_zoom(dock_window, mtm)?;

    let app_el = unsafe { AXUIElementCreateApplication(pid) };
    if app_el.is_null() {
        return Err("failed to create AXUIElement for application".to_string());
    }

    let window_el = ax_focused_window(app_el).ok_or_else(|| {
        unsafe {
            CFRelease(app_el as CfTypeRef);
        }
        "no focused window to zoom".to_string()
    })?;

    let current = match ax_window_frame_tauri(window_el, primary_cocoa_h, scale) {
        Ok(frame) => frame,
        Err(err) => {
            unsafe {
                CFRelease(window_el as CfTypeRef);
                CFRelease(app_el as CfTypeRef);
            }
            return Err(err);
        }
    };
    let storage_key = ax_window_storage_key(window_el, bundle_id);

    let result = if rects_approximately_equal(current, usable, ZOOM_FRAME_TOLERANCE_PX) {
        let saved = zoom_state
            .saved_frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&storage_key);
        if let Some(saved) = saved {
            set_ax_window_frame_tauri(
                window_el,
                ScreenRect {
                    x: saved.x,
                    y: saved.y,
                    width: saved.width,
                    height: saved.height,
                },
                primary_cocoa_h,
                scale,
            )
        } else {
            Err("window is already zoomed but no saved frame to restore".to_string())
        }
    } else {
        {
            let mut guard = zoom_state
                .saved_frames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.insert(
                storage_key,
                crate::commands::apps::SavedWindowFrame {
                    x: current.x,
                    y: current.y,
                    width: current.width,
                    height: current.height,
                },
            );
        }
        set_ax_window_frame_tauri(window_el, usable, primary_cocoa_h, scale)
    };

    unsafe {
        CFRelease(window_el as CfTypeRef);
        CFRelease(app_el as CfTypeRef);
    }

    result
}

/// Zooms the focused window of a running app to fill the screen area above
/// (or beside) the dock pill; toggles back to the pre-zoom frame on repeat.
#[cfg(target_os = "macos")]
pub fn zoom_app_above_dock(app: AppHandle, bundle_id: String) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let bundle_id_for_thread = bundle_id.clone();
    let app_for_thread = app.clone();

    app.run_on_main_thread(move || {
        let result = match app_for_thread.get_webview_window("main") {
            Some(dock_window) => {
                let zoom_state = app_for_thread.state::<crate::commands::apps::ZoomState>();
                zoom_app_above_dock_on_main_thread(
                    &dock_window,
                    &zoom_state,
                    &bundle_id_for_thread,
                )
            }
            None => Err("main window missing".to_string()),
        };
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    rx.recv()
        .map_err(|_| "zoom_app_above_dock did not complete".to_string())?
}

/// Activates the app if a running instance exists (brings its windows to
/// front, does not spawn a second instance); otherwise launches it.
/// Dispatched onto the main thread — `NSRunningApplication`/`NSWorkspace`
/// calls aren't safe from the Tauri command threadpool.
#[cfg(target_os = "macos")]
pub fn activate_or_launch_app(app: AppHandle, bundle_id: String) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(activate_or_launch_app_on_main_thread(
            &app_for_main,
            &bundle_id,
        ));
    })
    .map_err(|e| e.to_string())?;

    rx.recv()
        .map_err(|_| "activate_or_launch_app did not complete".to_string())?
}

/// Whether `pid` owns at least one on-screen window at layer 0 — used to
/// decide if a running app still needs `kAEReopenApplication` after
/// `activateWithOptions` (minimized windows are absent from this list).
#[cfg(target_os = "macos")]
fn process_has_on_screen_windows(pid: i32) -> bool {
    use core_foundation::base::{CFType, FromVoid, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
    };

    let option = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let Some(windows) = copy_window_info(option, kCGNullWindowID) else {
        return false;
    };

    let pid_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };

    for ptr in windows.get_all_values() {
        let dict = unsafe { CFDictionary::<CFString, CFType>::from_void(ptr) };
        let Some(owner_ref) = dict.find(&pid_key) else {
            continue;
        };
        let Some(owner) = owner_ref.downcast::<CFNumber>() else {
            continue;
        };
        if owner.to_i32() != Some(pid) {
            continue;
        }
        let Some(layer_ref) = dict.find(&layer_key) else {
            continue;
        };
        let Some(layer) = layer_ref.downcast::<CFNumber>() else {
            continue;
        };
        if layer.to_i32() == Some(0) {
            return true;
        }
    }

    false
}

/// Sends `kAEReopenApplication` — the same Apple Event the system Dock uses
/// to restore minimized windows or show a hidden app with no visible windows.
#[cfg(target_os = "macos")]
fn send_reopen_apple_event(pid: i32) -> Result<(), String> {
    use objc2_core_services::{kAEReopenApplication, kCoreEventClass};
    use objc2_foundation::NSAppleEventDescriptor;

    let target = NSAppleEventDescriptor::descriptorWithProcessIdentifier(pid);
    let event = NSAppleEventDescriptor::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
        kCoreEventClass,
        kAEReopenApplication,
        Some(&target),
        -1,
        0,
    );

    // `sendEventWithOptions` pulls in `NSDate`; `AESendMessage` on the raw
    // `AEDesc` keeps the dependency surface minimal.
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AESendMessage(
            the_apple_event: *const std::ffi::c_void,
            the_reply: *mut std::ffi::c_void,
            send_mode: u32,
            timeout_in_ticks: i32,
        ) -> i16;
    }

    const K_AE_NO_REPLY: u32 = 1;
    const K_AE_DEFAULT_TIMEOUT: i32 = -1;

    let status = unsafe {
        AESendMessage(
            event.aeDesc().cast(),
            std::ptr::null_mut(),
            K_AE_NO_REPLY,
            K_AE_DEFAULT_TIMEOUT,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(format!("AESendMessage(kAEReopenApplication) failed: {status}"))
    }
}

/// Brings a running instance to the foreground, unhiding and deminiaturizing
/// as needed — mirrors system Dock click on an already-launched app.
#[cfg(target_os = "macos")]
fn activate_running_application(
    instance: &objc2_app_kit::NSRunningApplication,
) -> Result<(), String> {
    use objc2_app_kit::NSApplicationActivationOptions;

    if instance.isHidden() {
        let _ = instance.unhide();
    }

    let pid = instance.processIdentifier();
    let _ = instance.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);

    if !process_has_on_screen_windows(pid) {
        send_reopen_apple_event(pid)?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn activate_or_launch_app_on_main_thread(
    app: &AppHandle,
    bundle_id: &str,
) -> Result<(), String> {
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};
    use objc2_foundation::NSString;

    let ns_bundle_id = NSString::from_str(bundle_id);
    let running = NSRunningApplication::runningApplicationsWithBundleIdentifier(&ns_bundle_id);

    if let Some(instance) = running.iter().find(|instance| !instance.isTerminated()) {
        let result = activate_running_application(&instance);
        if result.is_ok() {
            sync_bundle_running_state(app, bundle_id);
        }
        return result;
    }

    let workspace = NSWorkspace::sharedWorkspace();
    let app_url = workspace
        .URLForApplicationWithBundleIdentifier(&ns_bundle_id)
        .ok_or_else(|| format!("{bundle_id} is not installed"))?;
    let path = app_url.path().ok_or_else(|| "app URL has no path".to_string())?;

    let result =
        tauri_plugin_opener::open_path(path.to_string(), None::<&str>).map_err(|e| e.to_string());
    if result.is_ok() {
        sync_bundle_running_state(app, bundle_id);
        start_launch_running_watch(app.clone(), bundle_id.to_string());
    }
    result
}

/// Reveals the installed `.app` for `bundle_id` in Finder, with the icon
/// selected — same `URLForApplicationWithBundleIdentifier` resolve used for
/// launch/icon, just handed to Finder instead of opened or rasterized.
/// Dispatched onto the main thread for consistency with every other
/// `NSWorkspace`/AppKit call in this module (`activate_or_launch_app`,
/// `quit_app`), even though this particular call has not been observed to
/// require it.
#[cfg(target_os = "macos")]
pub fn reveal_app_in_finder(app: AppHandle, bundle_id: String) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(reveal_app_in_finder_on_main_thread(&bundle_id));
    })
    .map_err(|e| e.to_string())?;

    rx.recv()
        .map_err(|_| "reveal_app_in_finder did not complete".to_string())?
}

#[cfg(target_os = "macos")]
fn reveal_app_in_finder_on_main_thread(bundle_id: &str) -> Result<(), String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSString};

    let ns_bundle_id = NSString::from_str(bundle_id);
    let workspace = NSWorkspace::sharedWorkspace();
    let app_url = workspace
        .URLForApplicationWithBundleIdentifier(&ns_bundle_id)
        .ok_or_else(|| format!("{bundle_id} is not installed"))?;

    let urls = NSArray::from_slice(&[app_url.as_ref()]);
    workspace.activateFileViewerSelectingURLs(&urls);
    Ok(())
}

/// Soft-quits the running instance of `bundle_id`, if any. Uses `terminate()`
/// (asks the app to quit normally, respecting its own unsaved-changes
/// prompts etc.) — never `forceTerminate()`, which kills it outright. Does
/// **not** touch `AppsState.running` itself: the real LED update comes from
/// `NSWorkspaceDidTerminateApplicationNotification` via
/// `handle_workspace_notification`, once the app has actually exited, same
/// as a user quitting it any other way (Cmd+Q, Dock, Activity Monitor).
/// Dispatched onto the main thread — `NSRunningApplication` lookups aren't
/// safe from the Tauri command threadpool (same rationale as
/// `activate_or_launch_app`).
#[cfg(target_os = "macos")]
pub fn quit_app(app: AppHandle, bundle_id: String) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(quit_app_on_main_thread(&bundle_id));
    })
    .map_err(|e| e.to_string())?;

    rx.recv()
        .map_err(|_| "quit_app did not complete".to_string())?
}

#[cfg(target_os = "macos")]
fn quit_app_on_main_thread(bundle_id: &str) -> Result<(), String> {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    let ns_bundle_id = NSString::from_str(bundle_id);
    let running = NSRunningApplication::runningApplicationsWithBundleIdentifier(&ns_bundle_id);

    let Some(instance) = running.iter().next() else {
        return Err(format!("{bundle_id} is not running"));
    };

    if instance.terminate() {
        Ok(())
    } else {
        Err(format!("failed to send terminate to {bundle_id}"))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod geometry_tests {
    use super::*;
    use crate::commands::apps::MenuOverlaySide;

    #[test]
    fn axis_css_dims_maps_length_to_width_for_horizontal_dock() {
        let (w, h) = axis_css_dims(DockAxis::Horizontal, 500.0, 90.0);
        assert_eq!(w, 500.0);
        assert_eq!(h, 90.0);
    }

    #[test]
    fn axis_css_dims_maps_length_to_height_for_vertical_dock() {
        let (w, h) = axis_css_dims(DockAxis::Vertical, 500.0, 90.0);
        assert_eq!(w, 90.0);
        assert_eq!(h, 500.0);
    }

    #[test]
    fn add_remove_grows_width_on_bottom_dock_not_height() {
        let icon = 56.0;
        let pill_short = 400.0;
        let pill_long = 480.0;
        let thickness = 90.0;
        let window_short = window_length_dip(pill_short, icon);
        let window_long = window_length_dip(pill_long, icon);
        let (w_short, h_short) =
            axis_css_dims(DockAxis::Horizontal, window_short, window_thickness_dip(thickness, icon));
        let (w_long, h_long) =
            axis_css_dims(DockAxis::Horizontal, window_long, window_thickness_dip(thickness, icon));
        assert!(w_long > w_short);
        assert!((h_long - h_short).abs() < 0.5);
    }

    #[test]
    fn add_remove_grows_height_on_right_dock_not_width() {
        let icon = 56.0;
        let pill_short = 400.0;
        let pill_long = 480.0;
        let thickness = 90.0;
        let window_short = window_length_dip(pill_short, icon);
        let window_long = window_length_dip(pill_long, icon);
        let (w_short, h_short) =
            axis_css_dims(DockAxis::Vertical, window_short, window_thickness_dip(thickness, icon));
        let (w_long, h_long) =
            axis_css_dims(DockAxis::Vertical, window_long, window_thickness_dip(thickness, icon));
        assert!(h_long > h_short);
        assert!((w_long - w_short).abs() < 0.5);
    }

    #[test]
    fn menu_on_right_dock_extends_length_when_opened_to_side() {
        let (thickness_ext, length_ext) = menu_overlay_axis_extents(
            DockPosition::Right,
            MenuOverlaySide::Left,
            160.0,
            220.0,
        );
        assert!(thickness_ext > 0.0);
        assert_eq!(length_ext, 0.0);
    }

    #[test]
    fn menu_on_bottom_dock_extends_length_when_opened_to_side() {
        let (thickness_ext, length_ext) = menu_overlay_axis_extents(
            DockPosition::Bottom,
            MenuOverlaySide::Left,
            160.0,
            220.0,
        );
        assert_eq!(thickness_ext, 0.0);
        assert!(length_ext > 0.0);
    }

    #[test]
    fn shrink_visible_for_bottom_dock_stops_above_pill() {
        let visible = ScreenRect {
            x: 0,
            y: 50,
            width: 1920,
            height: 1030,
        };
        let shrunk = shrink_visible_for_dock(visible, 800, 980, 1120, 1070, DockPosition::Bottom, 4)
            .expect("should shrink");
        assert_eq!(shrunk.x, 0);
        assert_eq!(shrunk.y, 50);
        assert_eq!(shrunk.width, 1920);
        assert_eq!(shrunk.height, 980 - 50 - 4);
    }

    #[test]
    fn shrink_visible_for_top_dock_starts_below_pill() {
        let visible = ScreenRect {
            x: 0,
            y: 25,
            width: 1440,
            height: 875,
        };
        let shrunk = shrink_visible_for_dock(visible, 600, 25, 840, 120, DockPosition::Top, 4)
            .expect("should shrink");
        assert_eq!(shrunk.y, 124);
        assert_eq!(shrunk.height, 25 + 875 - 124);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod activation_tests {
    use super::*;
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    fn running_pid(bundle_id: &str) -> Option<i32> {
        let apps =
            NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
                bundle_id,
            ));
        apps.iter().next().map(|app| app.processIdentifier())
    }

    #[test]
    fn process_has_on_screen_windows_does_not_panic_for_finder() {
        if let Some(pid) = running_pid("com.apple.finder") {
            let _ = process_has_on_screen_windows(pid);
        }
    }

    /// Minimize TextEdit first: `osascript -e 'tell application "TextEdit" to set miniaturized of window 1 to true'`
    #[test]
    #[ignore = "manual: requires TextEdit running with a minimized window"]
    fn reopen_restores_minimized_textedit() {
        let Some(pid) = running_pid("com.apple.TextEdit") else {
            panic!("TextEdit is not running");
        };
        assert!(
            !process_has_on_screen_windows(pid),
            "window 1 must be minimized before running this test"
        );

        send_reopen_apple_event(pid).expect("kAEReopenApplication should succeed");
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(
            process_has_on_screen_windows(pid),
            "TextEdit window should be visible after reopen"
        );
    }

    /// Launch TextEdit with a document before running.
    #[test]
    #[ignore = "manual: requires TextEdit running"]
    fn activate_running_application_unhides_textedit() {
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &NSString::from_str("com.apple.TextEdit"),
        );
        let Some(instance) = apps.iter().next() else {
            panic!("TextEdit is not running");
        };

        let _ = instance.hide();
        std::thread::sleep(std::time::Duration::from_millis(500));

        activate_running_application(&instance).expect("activation should succeed");
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(
            process_has_on_screen_windows(instance.processIdentifier()),
            "TextEdit should be visible after activation"
        );
    }
}
