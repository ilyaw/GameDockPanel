use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{App, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::commands::apps::AppsState;

/// Cursor position in webview logical (DIP) coords — emitted while the pointer
/// is over the dock pill so React can hit-test icons without CSS :hover.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct DockCursorPayload {
    x: f64,
    y: f64,
}

// Layout formula mirrored from src/lib/constants.ts — see pillWidthPx()/
// windowWidthDip() there for the JS-side copy of this math. DIP == points
// throughout (Tauri's logical-pixel convention), so these are used directly
// against `NSRect`/`NSWindow` without any extra unit conversion.
const ICON_SIZE_DIP: f64 = 56.0;
// Mirrors `gap-2` on the dock pill (src/lib/constants.ts DOCK_GAP_PX).
const DOCK_GAP_DIP: f64 = 8.0;
const DOCK_PADDING_X_DIP: f64 = 20.0;
const MAGNIFY_MAX_SCALE: f64 = 1.4;
const WINDOW_GLOW_BLEED_DIP: f64 = 32.0;

// Keep in sync with WINDOW_HEIGHT_DIP / PILL_* in src/lib/constants.ts.
// Height never depends on app count — only width does (see pill_width_dip()).
// 194 = DOCK_BOTTOM_INSET_DIP(8) + PILL_HEIGHT_REST_DIP(91) +
// PILL_TOP_RESERVE_PX(95) — the top reserve is sized for the tallest thing
// that ever pokes above the pill, which is now DockIcon's context menu
// (Show in Finder + Remove from Dock + divider + Quit), not the shorter hover
// tooltip or magnify overflow. See PILL_TOP_RESERVE_PX's `Math.max(...)` in
// constants.ts for the full derivation — this constant must track it.
const WINDOW_HEIGHT_DIP: f64 = 194.0;
const PILL_HEIGHT_REST_DIP: f64 = 91.0;
const PILL_HEIGHT_HOVER_DIP: f64 = 114.0;
const DOCK_BOTTOM_INSET_DIP: f64 = 8.0;
/// Must match Tailwind's `rounded-[28px]` on the dock pill (DockPanel.tsx) —
/// CSS and the native vibrancy/hit-test masks below only agree with the
/// visible shape if this stays in sync with that class.
const PILL_CORNER_RADIUS_DIP: f64 = 28.0;
const CLICK_POLL_MS: u64 = 50;
/// Mirrors `TOOLTIP_GAP_PX` in src/lib/constants.ts — the gap between the
/// open context menu's own bottom edge and the icon it hangs off of. Used
/// to extend the click-through hit-test up through that gap and into the
/// menu itself (see `AppsState::menu_overlay_height_dip`).
const MENU_OVERLAY_GAP_DIP: f64 = 16.0;

/// Raster size for native icon export — `NSWorkspace.iconForFile` defaults to
/// 32×32; upscaling that in the dock looks blocky. Keep ≥ `ICON_SIZE_PX *
/// MAGNIFY_MAX_SCALE * 2` from `src/lib/constants.ts` (56 × 1.4 × 2 ≈ 157).
const ICON_EXPORT_PX: f64 = 256.0;

fn pill_width_dip(app_count: usize) -> f64 {
    DOCK_PADDING_X_DIP * 2.0
        + app_count as f64 * ICON_SIZE_DIP
        + app_count.saturating_sub(1) as f64 * DOCK_GAP_DIP
}

fn window_width_dip(pill_width_dip: f64) -> f64 {
    pill_width_dip + (ICON_SIZE_DIP * (MAGNIFY_MAX_SCALE - 1.0)).ceil() + WINDOW_GLOW_BLEED_DIP
}

/// Positions, sizes and reveals the main window: a compact, always-on-top
/// strip anchored to the bottom-center of the primary display, with the
/// app hidden from the Dock. Initial width is computed from
/// `AppsState.entries` (populated by `commands::apps::init_entries` just
/// before this runs), not a fixed constant.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let app_count = app.state::<AppsState>().app_count();
    let pill_width = pill_width_dip(app_count);
    let window_width = window_width_dip(pill_width);

    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no primary monitor".to_string())?;

    let scale = monitor.scale_factor();
    let monitor_size = *monitor.size();
    let monitor_pos = *monitor.position();
    let width = (window_width * scale).round() as i32;
    let height = (WINDOW_HEIGHT_DIP * scale).round() as i32;

    window
        .set_size(PhysicalSize::new(width as u32, height as u32))
        .map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(
            monitor_pos.x + (monitor_size.width as i32 - width) / 2,
            monitor_pos.y + monitor_size.height as i32 - height,
        ))
        .map_err(|e| e.to_string())?;
    window
        .set_always_on_top(true)
        .map_err(|e| e.to_string())?;

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
    }

    window.show().map_err(|e| e.to_string())?;

    enable_inactive_mouse_tracking(&window)?;
    apply_dock_vibrancy(&window, pill_width)?;

    start_dock_click_tap(window.clone());
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
fn apply_dock_vibrancy(window: &WebviewWindow, pill_width_dip: f64) -> Result<(), String> {
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

    set_vibrancy_pill_frame(window, pill_width_dip, PILL_HEIGHT_REST_DIP, None, None)
}

/// Resizes the masked vibrancy blur view to the given pill footprint.
/// When `origin_x` / `origin_y` are `None`, the frame is centered on X and
/// anchored with `DOCK_BOTTOM_INSET_DIP` on Y (startup before DOM measure).
/// When provided, values are webview logical coords from
/// `getBoundingClientRect()` (see `sync_vibrancy_pill_from_web`).
#[cfg(target_os = "macos")]
fn set_vibrancy_pill_frame(
    window: &WebviewWindow,
    width_dip: f64,
    height_dip: f64,
    origin_x: Option<f64>,
    origin_y: Option<f64>,
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

    blur_view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);

    let mut pill_frame = bounds;
    pill_frame.size.width = width_dip;
    pill_frame.size.height = height_dip;
    pill_frame.origin.x = origin_x.unwrap_or((bounds.size.width - width_dip) / 2.0);
    pill_frame.origin.y = match origin_y {
        Some(y) if parent.isFlipped() => y,
        Some(y) => bounds.size.height - y - height_dip,
        None if parent.isFlipped() => bounds.size.height - DOCK_BOTTOM_INSET_DIP - height_dip,
        None => DOCK_BOTTOM_INSET_DIP,
    };

    blur_view.setClipsToBounds(true);
    blur_view.setFrame(pill_frame);

    Ok(())
}

/// Positions the vibrancy blur view from the pill's measured DOM rect.
#[cfg(target_os = "macos")]
pub fn sync_vibrancy_pill_from_web(
    window: &WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    set_vibrancy_pill_frame(window, width, height, Some(x), Some(y))?;

    let state = window.state::<AppsState>();
    let mut current_width = state
        .pill_width_dip
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *current_width = width;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_dock_vibrancy(_window: &WebviewWindow, _pill_width_dip: f64) -> Result<(), String> {
    Ok(())
}

/// Applies a discrete pill-width change for an already-visible window: one
/// coordinated pass over NSWindow frame (animated), vibrancy pill frame,
/// and the shared width value hit-testing reads. Only ever called from
/// `add_app_from_path` / `remove_app` — a rare, user-initiated, discrete
/// event — never from hover/magnify/mousemove paths. Height never changes
/// here, only width.
#[cfg(target_os = "macos")]
pub fn sync_dock_geometry(window: &WebviewWindow, app_count: usize) -> Result<(), String> {
    use objc2_app_kit::{NSAnimatablePropertyContainer, NSAnimationContext, NSView, NSWindow};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let pill_width = pill_width_dip(app_count);
    let window_width = window_width_dip(pill_width);

    let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())? as *mut NSWindow;
    let ns_window = unsafe { &*ns_window_ptr };

    // Only width changes on this path — derive the new frame from the
    // window's *current* frame (already correct AppKit bottom-left-origin
    // coordinates) instead of recomputing from monitor geometry, so no
    // Y-flip math is needed: origin.y and size.height carry over untouched,
    // only origin.x and size.width move, symmetric around the old center.
    let current_frame = ns_window.frame();
    let center_x = current_frame.origin.x + current_frame.size.width / 2.0;
    let new_frame = NSRect::new(
        NSPoint::new(center_x - window_width / 2.0, current_frame.origin.y),
        NSSize::new(window_width, current_frame.size.height),
    );

    let ns_view_ptr = window.ns_view().map_err(|e| e.to_string())? as *mut NSView;
    let ns_view = unsafe { &*ns_view_ptr };
    let blur_view = ns_view.viewWithTag(VIBRANCY_VIEW_TAG);

    // Derived straight from `window_width` — not from the blur view's
    // `superview().bounds()` the way `set_vibrancy_pill_frame(..., None,
    // None)` would, since those bounds won't reflect the new width until
    // the frame change below actually lands on screen.
    let new_pill_frame = NSRect::new(
        NSPoint::new((window_width - pill_width) / 2.0, DOCK_BOTTOM_INSET_DIP),
        NSSize::new(pill_width, PILL_HEIGHT_REST_DIP),
    );

    // Animate the window frame and the vibrancy blur view's frame in the
    // same `NSAnimationContext` group so they move in lockstep. Previously
    // only the window frame animated (`setFrame_display_animate`) while the
    // blur view snapped to its end state instantly — for the whole
    // animation the visible glass pill (smaller, already centered) and the
    // window's actual bounds (still mid-resize) disagreed, which is what
    // left a stray gap/flash behind after adding or removing an app.
    NSAnimationContext::runAnimationGroup(&block2::RcBlock::new(move |_context| {
        ns_window.animator().setFrame_display(new_frame, true);
        if let Some(view) = &blur_view {
            view.animator().setFrame(new_pill_frame);
        }
    }));

    {
        let state = window.state::<AppsState>();
        let mut current_width = state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current_width = pill_width;
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn sync_dock_geometry(_window: &WebviewWindow, _app_count: usize) -> Result<(), String> {
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

/// Maps a screen-space cursor position to DIP coords inside the window when
/// the point lies on the rounded pill footprint; `None` otherwise. Reads
/// the pill's current width from `AppsState` (mutated by `setup_dock_window`
/// / `sync_dock_geometry`) instead of a compile-time constant, so both
/// consumers below (the click tap and the hover poller) automatically
/// hit-test against whatever width is currently applied.
#[cfg(target_os = "macos")]
fn pill_cursor_at_screen(
    window: &WebviewWindow,
    screen_x: i32,
    screen_y: i32,
    pill_height_dip: f64,
) -> Option<DockCursorPayload> {
    let scale = window.scale_factor().ok()?;
    let outer_pos = window.outer_position().ok()?;
    let outer_size = window.outer_size().ok()?;
    let pill_width_dip = {
        let state = window.state::<AppsState>();
        let guard = state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard
    };

    let pill_w = (pill_width_dip * scale).round() as i32;
    let pill_h = (pill_height_dip * scale).round() as i32;
    let inset = (DOCK_BOTTOM_INSET_DIP * scale).round() as i32;
    let radius = (PILL_CORNER_RADIUS_DIP * scale).round() as i32;

    let pill_left = outer_pos.x + (outer_size.width as i32 - pill_w) / 2;
    let pill_top = outer_pos.y + outer_size.height as i32 - inset - pill_h;
    let pill_right = pill_left + pill_w;
    let pill_bottom = pill_top + pill_h;

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

    std::thread::spawn(move || {
        let mach_port: Arc<Mutex<Option<CFMachPort>>> = Arc::new(Mutex::new(None));
        let mach_port_cb = Arc::clone(&mach_port);
        let pending_down: Arc<Mutex<Option<PendingDown>>> = Arc::new(Mutex::new(None));
        let pending_down_cb = Arc::clone(&pending_down);

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
                                if let Some(payload) = pill_cursor_at_screen(
                                    &window,
                                    cursor_x,
                                    cursor_y,
                                    PILL_HEIGHT_HOVER_DIP,
                                ) {
                                    let _ = window.emit("dock-click", payload);
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

            let menu_overlay_height_dip = {
                let state = window.state::<AppsState>();
                let guard = state
                    .menu_overlay_height_dip
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *guard
            };

            // While a `DockIcon` context menu is open, the hit-test rect has
            // to reach all the way up through it — the menu can render much
            // taller than the fixed magnify-overflow band below, and any
            // shorter test here means the OS re-engages click-through under
            // the cursor before it reaches the upper menu rows, making them
            // permanently unreachable (see `AppsState::menu_overlay_height_dip`).
            let pill_h_dip = if menu_overlay_height_dip > 0.0 {
                PILL_HEIGHT_REST_DIP + MENU_OVERLAY_GAP_DIP + menu_overlay_height_dip
            } else if dock_hovered {
                PILL_HEIGHT_HOVER_DIP
            } else {
                PILL_HEIGHT_REST_DIP
            };
            let pill_cursor = pill_cursor_at_screen(&window, cursor_x, cursor_y, pill_h_dip);
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
        let running_apps = workspace.runningApplications();
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for entry in entries.iter() {
            let is_running = running_apps.iter().any(|running_app| {
                running_app
                    .bundleIdentifier()
                    .map(|bundle_id| bundle_id.to_string() == entry.bundle_id)
                    .unwrap_or(false)
            });
            running.insert(entry.bundle_id.clone(), is_running);
        }
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut icons = state
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for entry in entries.iter() {
            icons.insert(entry.bundle_id.clone(), resolve_icon_data_url(&entry.bundle_id));
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

    emit_apps_icons_updated(&app_handle, app.state::<AppsState>().icons_snapshot());
    emit_apps_running_changed(&app_handle);

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
        entries.iter().any(|entry| entry.bundle_id == bundle_id)
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
            if let Some(icon_url) = resolve_icon_data_url(&bundle_id) {
                let mut icons = state
                    .icons
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                icons.insert(bundle_id.clone(), Some(icon_url));
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

#[cfg(target_os = "macos")]
fn emit_apps_icons_updated(app: &AppHandle, updates: Vec<crate::commands::apps::AppIconUpdatePayload>) {
    let _ = app.emit("apps-icons-updated", updates);
}

/// Resolves a bundle ID to its installed `.app`'s icon and renders it as a
/// `data:image/png;base64,...` URL. Returns `None` if the app isn't
/// installed or the icon can't be encoded — the frontend falls back to an
/// initials badge either way, same as a failed remote image load. Exposed
/// to `commands::apps` (as `resolve_app_icon`) via `platform::mod` for the
/// add-app-from-dialog flow.
#[cfg(target_os = "macos")]
pub fn resolve_icon_data_url(bundle_id: &str) -> Option<String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    let workspace = NSWorkspace::sharedWorkspace();
    let ns_bundle_id = NSString::from_str(bundle_id);
    let app_url = workspace.URLForApplicationWithBundleIdentifier(&ns_bundle_id)?;
    let path = app_url.path()?;
    let icon = workspace.iconForFile(&path);
    icon_to_png_data_url(&icon)
}

/// `NSImage` → PNG bytes via `CGImage`, not manual `.icns`/`Info.plist`
/// parsing (see PROMPT_04_PROCESS_MONITORING.md point 6). `NSImage` doesn't
/// expose a modern direct-to-PNG method, so the standard AppKit route is
/// `NSImage` → `CGImage` → `NSBitmapImageRep` → PNG `NSData`.
#[cfg(target_os = "macos")]
fn icon_to_png_data_url(icon: &objc2_app_kit::NSImage) -> Option<String> {
    use base64::Engine as _;
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

    // Pick the best embedded representation (512/256/128…) for dock display.
    let export_size = NSSize::new(ICON_EXPORT_PX, ICON_EXPORT_PX);
    icon.setSize(export_size);
    let mut proposed_rect = NSRect::new(NSPoint::new(0.0, 0.0), export_size);
    let cg_image =
        unsafe { icon.CGImageForProposedRect_context_hints(&mut proposed_rect, None, None) }?;

    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg_image);
    let properties = NSDictionary::new();
    let png_data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }?;

    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png_data.to_vec())
    ))
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
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    let ns_bundle_id = NSString::from_str(bundle_id);
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&ns_bundle_id)
        .iter()
        .next()
        .is_some()
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

/// Activates the app if a running instance exists (brings its windows to
/// front, does not spawn a second instance); otherwise launches it.
/// Dispatched onto the main thread — `NSRunningApplication`/`NSWorkspace`
/// calls aren't safe from the Tauri command threadpool.
#[cfg(target_os = "macos")]
pub fn activate_or_launch_app(app: AppHandle, bundle_id: String) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(activate_or_launch_app_on_main_thread(&bundle_id));
    })
    .map_err(|e| e.to_string())?;

    rx.recv()
        .map_err(|_| "activate_or_launch_app did not complete".to_string())?
}

#[cfg(target_os = "macos")]
fn activate_or_launch_app_on_main_thread(bundle_id: &str) -> Result<(), String> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
    use objc2_foundation::NSString;

    let ns_bundle_id = NSString::from_str(bundle_id);
    let running = NSRunningApplication::runningApplicationsWithBundleIdentifier(&ns_bundle_id);

    if let Some(instance) = running.iter().next() {
        if instance.activateWithOptions(NSApplicationActivationOptions::empty()) {
            return Ok(());
        }
        // Instance was listed as running but activation failed (e.g. quit race) — launch below.
    }

    let workspace = NSWorkspace::sharedWorkspace();
    let app_url = workspace
        .URLForApplicationWithBundleIdentifier(&ns_bundle_id)
        .ok_or_else(|| format!("{bundle_id} is not installed"))?;
    let path = app_url.path().ok_or_else(|| "app URL has no path".to_string())?;

    tauri_plugin_opener::open_path(path.to_string(), None::<&str>).map_err(|e| e.to_string())
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
