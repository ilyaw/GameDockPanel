mod commands;
mod persistence;
mod platform;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(commands::apps::AppsState::default())
        .manage(commands::apps::ZoomState::default())
        .manage(commands::settings::SettingsState::default())
        .invoke_handler(tauri::generate_handler![
            commands::apps::get_apps_snapshot,
            commands::apps::launch_or_activate_app,
            commands::apps::zoom_app_above_dock,
            commands::apps::add_app_from_path,
            commands::apps::remove_app,
            commands::apps::reorder_apps,
            commands::apps::insert_separator,
            commands::apps::remove_separator,
            commands::apps::reveal_app_in_finder,
            commands::apps::quit_app,
            commands::apps::set_app_indicator_color,
            commands::apps::refresh_indicator_colors,
            commands::window::resize_dock_window,
            commands::window::sync_vibrancy_pill,
            commands::window::set_menu_overlay,
            commands::window::open_settings,
            commands::settings::get_dock_settings,
            commands::settings::update_dock_settings,
            #[cfg(debug_assertions)]
            commands::settings::qa_set_border,
        ])
        .setup(|app| {
            // Populate AppsState.entries (persisted list, or first-run seed)
            // before sizing the window — setup_dock_window reads the entry
            // count to compute its initial width.
            commands::apps::init_entries(app.handle())?;
            commands::settings::init_settings(app.handle())?;
            #[cfg(debug_assertions)]
            commands::settings::start_border_qa_poller(app.handle());
            platform::setup_dock_window(app)?;
            platform::start_apps_monitoring(app)?;
            tray::setup(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
