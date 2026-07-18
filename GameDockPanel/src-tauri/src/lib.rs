mod commands;
mod persistence;
mod platform;
mod tray;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin({
            let mut builder = tauri_plugin_log::Builder::new();
            #[cfg(debug_assertions)]
            {
                builder = builder.target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ));
            }
            builder
                // Debug so [win-backdrop] / geometry details land in release
                // builds too — friend ships logs for remote triage.
                .level(log::LevelFilter::Debug)
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("gamedockpanel".into()),
                    },
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Webview,
                ))
                .build()
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
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
            commands::window::set_dock_region_relaxed,
            commands::window::set_menu_overlay,
            commands::window::open_settings,
            commands::settings::get_dock_settings,
            commands::settings::update_dock_settings,
            commands::diagnostics::get_diagnostics,
            commands::diagnostics::open_log_dir,
            #[cfg(debug_assertions)]
            commands::settings::qa_set_border,
        ])
        .setup(|app| {
            log::info!(
                "GameDockPanel starting: os={} arch={} version={} debug={}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                app.package_info().version,
                cfg!(debug_assertions)
            );
            if let Ok(dir) = app.path().app_data_dir() {
                log::info!("app_data_dir={}", dir.display());
            }
            if let Ok(dir) = app.path().app_log_dir() {
                log::info!("log_dir={}", dir.display());
            }

            commands::apps::init_entries(app.handle())?;
            commands::settings::init_settings(app.handle())?;
            #[cfg(debug_assertions)]
            commands::settings::start_border_qa_poller(app.handle());
            platform::setup_dock_window(app)?;
            platform::start_apps_monitoring(app)?;
            tray::setup(app)?;
            log::info!("GameDockPanel setup complete");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
