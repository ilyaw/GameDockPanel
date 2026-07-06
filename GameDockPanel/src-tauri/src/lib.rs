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
        ])
        .setup(|app| {
            platform::setup_dock_window(app)?;
            platform::start_apps_monitoring(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
