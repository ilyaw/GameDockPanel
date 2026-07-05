use tauri::{App, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

// Keep in sync with WINDOW_*_DIP / PILL_* in src/lib/constants.ts.
const WINDOW_WIDTH_DIP: f64 = 511.0;
const WINDOW_HEIGHT_DIP: f64 = 127.0;
const PILL_WIDTH_DIP: f64 = 456.0;
const PILL_HEIGHT_DIP: f64 = 95.0;
const DOCK_BOTTOM_INSET_DIP: f64 = 20.0;
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

    start_click_through_poller(window);

    Ok(())
}

/// Polls the global cursor and toggles `set_ignore_cursor_events` so only the
/// dock pill captures input — transparent bands above it stay click-through
/// at the OS level (WKWebView `pointer-events-none` is not sufficient alone).
#[cfg(target_os = "macos")]
fn start_click_through_poller(window: WebviewWindow) {
    std::thread::spawn(move || {
        let mut ignoring = true;

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
