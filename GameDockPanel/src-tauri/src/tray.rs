//! Menu bar / system tray icon. Cross-platform Tauri API (`TrayIconBuilder`,
//! `Menu`) — not OS-specific mechanics, so this lives alongside `commands/`
//! and `platform/` rather than inside either (see dockpanel rule: platform-
//! specific code only in `platform/`; this isn't that).

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::App;

use crate::commands::window::open_settings_window;

/// Bytes for the macOS template tray icon — a monochrome (black + alpha)
/// glyph echoing the dock's own row-of-app-icons look. `icon_as_template(true)`
/// below lets macOS repaint it for the current dark/light menu bar; a
/// colored icon would look wrong in that mode (see `icon_as_template` docs).
#[cfg(target_os = "macos")]
const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/tray-icon.png");

/// `icon_as_template` is macOS-only (Tauri docs: no-op on Windows/Linux) —
/// the monochrome glyph above would render as flat black there, invisible
/// against the default dark taskbar. Use the full-color product mark
/// instead, which is the norm for Windows tray icons regardless of taskbar
/// theme or accent color.
#[cfg(not(target_os = "macos"))]
const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/32x32.png");

/// Builds the tray icon: left-click opens/focuses the settings window,
/// right-click shows a short menu (`Настройки`, `Выход`;
/// `show_menu_on_left_click(false)` keeps left-click free for the primary
/// action instead of popping the menu, which is Tauri's default).
pub fn setup(app: &App) -> Result<(), String> {
    let settings_item =
        MenuItem::with_id(app, "settings", "Настройки", true, None::<&str>)
            .map_err(|e| e.to_string())?;
    let quit_item = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&settings_item, &quit_item]).map_err(|e| e.to_string())?;

    let icon = tauri::image::Image::from_bytes(TRAY_ICON_BYTES).map_err(|e| e.to_string())?;
    log::info!(
        "[tray] icon bytes len={} template={}",
        TRAY_ICON_BYTES.len(),
        cfg!(target_os = "macos"),
    );

    #[allow(unused_mut)]
    let mut builder = TrayIconBuilder::with_id("main_tray")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("GameDockPanel");
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon_as_template(true);
    }

    builder
        .on_tray_icon_event(|tray, event| {
            log::debug!("[tray] icon event: {event:?}");
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Err(err) = open_settings_window(app) {
                    eprintln!("GameDockPanel: failed to open settings window: {err}");
                }
            }
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "settings" {
                if let Err(err) = open_settings_window(app) {
                    eprintln!("GameDockPanel: failed to open settings window: {err}");
                }
            } else if event.id().as_ref() == "quit" {
                app.exit(0);
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;

    Ok(())
}
