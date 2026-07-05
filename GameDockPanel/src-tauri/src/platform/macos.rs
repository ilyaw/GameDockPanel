use tauri::{App, Manager, PhysicalPosition, PhysicalSize};

/// Logical (DIP) footprint of the dock's host window: the pill itself plus
/// headroom for the hover-name tooltip and the future hover-magnify scale-up
/// (see `MAGNIFY_MAX_SCALE` in `src/lib/constants.ts`). Sized once, generously,
/// so the magnify pass won't need a dynamic window resize later — see the
/// "Размер окна" decision in the dock foundation plan.
const WINDOW_WIDTH_DIP: f64 = 720.0;
const WINDOW_HEIGHT_DIP: f64 = 240.0;

/// Positions, sizes and reveals the main window: a compact, always-on-top
/// strip anchored to the bottom-center of the primary display, with the
/// app hidden from the Dock.
pub fn setup_dock_window(app: &mut App) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    // `skipTaskbar` in tauri.conf.json is a no-op on macOS (there's no
    // taskbar concept) — this is the actual way to hide the Dock icon.
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

    // Real frosted glass (window_vibrancy::apply_vibrancy) is intentionally
    // deferred — see the plan's "Vibrancy vs CSS-тонирование" decision: this
    // window carries headroom around the pill for the tooltip and future
    // magnify, so a plain apply_vibrancy call would frost that whole
    // rectangle instead of just the pill. Revisit once a masked
    // NSVisualEffectView (or a tightly-fit window) makes sense — CSS
    // `bg-zinc-950/80` + `backdrop-blur-xl` carries the glass look for now.

    window.show().map_err(|e| e.to_string())?;

    Ok(())
}
