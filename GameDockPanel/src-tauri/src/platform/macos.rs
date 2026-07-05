use serde::Serialize;
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

    start_click_through_poller(window);

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

            let Ok(scale) = window.scale_factor() else {
                continue;
            };
            let Ok(outer_pos) = window.outer_position() else {
                continue;
            };
            let Ok(outer_size) = window.outer_size() else {
                continue;
            };
            let Some((cursor_x, cursor_y)) = global_cursor_position() else {
                continue;
            };

            let pill_w = (PILL_WIDTH_DIP * scale).round() as i32;
            let pill_h = (PILL_HEIGHT_DIP * scale).round() as i32;
            let inset = (DOCK_BOTTOM_INSET_DIP * scale).round() as i32;

            let pill_left = outer_pos.x + (outer_size.width as i32 - pill_w) / 2;
            let pill_top = outer_pos.y + outer_size.height as i32 - inset - pill_h;
            let pill_right = pill_left + pill_w;
            let pill_bottom = pill_top + pill_h;

            let in_pill = cursor_x >= pill_left
                && cursor_x <= pill_right
                && cursor_y >= pill_top
                && cursor_y <= pill_bottom;

            if in_pill != dock_hovered {
                dock_hovered = in_pill;
                let _ = window.emit("dock-hover", dock_hovered);
                if !dock_hovered {
                    last_cursor = None;
                }
            }

            if in_pill {
                let cursor = DockCursorPayload {
                    x: (cursor_x - outer_pos.x) as f64 / scale,
                    y: (cursor_y - outer_pos.y) as f64 / scale,
                };
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

#[cfg(target_os = "macos")]
fn global_cursor_position() -> Option<(i32, i32)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let event = CGEvent::new(source).ok()?;
    let loc = event.location();
    Some((loc.x.round() as i32, loc.y.round() as i32))
}
