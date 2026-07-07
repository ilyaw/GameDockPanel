mod commands;
mod platform;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(commands::apps::AppsState::default())
        .invoke_handler(tauri::generate_handler![
            commands::apps::get_apps_snapshot,
            commands::apps::launch_or_activate_app,
            commands::apps::add_app_from_path,
            commands::apps::remove_app,
            commands::apps::reorder_apps,
            commands::apps::reveal_app_in_finder,
            commands::apps::quit_app,
            commands::window::sync_vibrancy_pill,
        ])
        .setup(|app| {
            // Populate AppsState.entries (persisted list, or first-run seed)
            // before sizing the window — setup_dock_window reads the entry
            // count to compute its initial width.
            commands::apps::init_entries(app.handle())?;
            platform::setup_dock_window(app)?;
            platform::start_apps_monitoring(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
