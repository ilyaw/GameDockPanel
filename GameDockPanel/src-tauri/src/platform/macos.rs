use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{App, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// Cursor position in webview logical (DIP) coords — emitted while the pointer
/// is over the dock pill so React can hit-test icons without CSS :hover.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct DockCursorPayload {
    x: f64,
    y: f64,
}

// Keep in sync with WINDOW_*_DIP / PILL_* in src/lib/constants.ts.
const WINDOW_WIDTH_DIP: f64 = 511.0;
const WINDOW_HEIGHT_DIP: f64 = 111.0;
const PILL_WIDTH_DIP: f64 = 456.0;
const PILL_HEIGHT_DIP: f64 = 91.0;
const DOCK_BOTTOM_INSET_DIP: f64 = 8.0;
/// Must match Tailwind's `rounded-[28px]` on the dock pill (DockPanel.tsx) —
/// CSS and the native vibrancy/hit-test masks below only agree with the
/// visible shape if this stays in sync with that class.
const PILL_CORNER_RADIUS_DIP: f64 = 28.0;
const CLICK_POLL_MS: u64 = 50;

/// Positions, sizes and reveals the main window: a compact, always-on-top
/// strip anchored to the bottom-center of the primary display, with the
/// app hidden from the Dock.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no primary monitor".to_string())?;

    let scale = monitor.scale_factor();
    let monitor_size = *monitor.size();
    let monitor_pos = *monitor.position();
    let width = (WINDOW_WIDTH_DIP * scale).round() as i32;
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

    window.show().map_err(|e| e.to_string())?;

    enable_inactive_mouse_tracking(&window)?;
    apply_dock_vibrancy(&window)?;

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
/// rounded footprint. `apply_vibrancy()` always sizes its
/// `NSVisualEffectView` to the whole window content view (511x111 DIP),
/// which is bigger than the visible 456x91 DIP pill — the extra space is
/// reserved for RGB-glow bleed and future magnify/tooltip overflow (see
/// WINDOW_*_DIP vs PILL_*_DIP above). Left untouched, vibrancy would paint a
/// rectangular blur halo outside the pill's `rounded-[28px]` corners. So
/// after applying vibrancy we look the blur view back up by its internal
/// tag and shrink its frame down to just the pill's rect — the crate's own
/// `radius` argument then rounds that smaller rect correctly.
#[cfg(target_os = "macos")]
fn apply_dock_vibrancy(window: &WebviewWindow) -> Result<(), String> {
    use objc2_app_kit::NSWindow;
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

    let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())? as *mut NSWindow;
    let ns_window = unsafe { &*ns_window_ptr };
    let content_view = ns_window
        .contentView()
        .ok_or_else(|| "no content view".to_string())?;

    let Some(blur_view) = content_view.viewWithTag(VIBRANCY_VIEW_TAG) else {
        return Ok(()); // best-effort — vibrancy still shows, just unmasked
    };

    // Frame must be in the blur view's superview coords (the WKWebView), not
    // the window content view — apply_vibrancy attaches it there.
    let parent = unsafe {
        blur_view
            .superview()
            .ok_or_else(|| "blur view has no superview".to_string())?
    };
    let bounds = parent.bounds();
    let mut pill_frame = bounds;
    pill_frame.size.width = PILL_WIDTH_DIP;
    pill_frame.size.height = PILL_HEIGHT_DIP;
    pill_frame.origin.x = (bounds.size.width - PILL_WIDTH_DIP) / 2.0;
    pill_frame.origin.y = if parent.isFlipped() {
        bounds.size.height - DOCK_BOTTOM_INSET_DIP - PILL_HEIGHT_DIP
    } else {
        DOCK_BOTTOM_INSET_DIP
    };
    blur_view.setFrame(pill_frame);

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_dock_vibrancy(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// Installs an `NSTrackingActiveAlways` area on the window content view so
/// mouse-enter/move events reach the WebView even when another app is key.
/// Does not call `makeKeyWindow` — the dock must not steal focus on hover.
#[cfg(target_os = "macos")]
fn enable_inactive_mouse_tracking(window: &WebviewWindow) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSWindow, NSTrackingArea, NSTrackingAreaOptions};

    let ns_window_ptr = window
        .ns_window()
        .map_err(|e| e.to_string())? as *mut NSWindow;
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

#[cfg(not(target_os = "macos"))]
fn enable_inactive_mouse_tracking(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// Maps a screen-space cursor position to DIP coords inside the window when
/// the point lies on the rounded pill footprint; `None` otherwise.
#[cfg(target_os = "macos")]
fn pill_cursor_at_screen(
    window: &WebviewWindow,
    screen_x: i32,
    screen_y: i32,
) -> Option<DockCursorPayload> {
    let scale = window.scale_factor().ok()?;
    let outer_pos = window.outer_position().ok()?;
    let outer_size = window.outer_size().ok()?;

    let pill_w = (PILL_WIDTH_DIP * scale).round() as i32;
    let pill_h = (PILL_HEIGHT_DIP * scale).round() as i32;
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

/// Event-driven `LeftMouseDown` capture at the HID layer — reliable for short
/// trackpad taps and for clicks swallowed by the vibrancy `NSVisualEffectView`
/// before they reach WKWebView / React.
#[cfg(target_os = "macos")]
fn start_dock_click_tap(window: WebviewWindow) {
    use core_foundation::mach_port::CFMachPort;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    };

    std::thread::spawn(move || {
        let mach_port: Arc<Mutex<Option<CFMachPort>>> = Arc::new(Mutex::new(None));
        let mach_port_cb = Arc::clone(&mach_port);

        let tap = match CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            vec![CGEventType::LeftMouseDown],
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
                            let cursor_x = cursor.x.round() as i32;
                            let cursor_y = cursor.y.round() as i32;
                            if let Some(payload) =
                                pill_cursor_at_screen(&window, cursor_x, cursor_y)
                            {
                                let _ = window.emit("dock-click", payload);
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

            let pill_cursor = pill_cursor_at_screen(&window, cursor_x, cursor_y);
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
