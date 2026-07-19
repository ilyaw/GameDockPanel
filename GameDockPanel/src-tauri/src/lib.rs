mod commands;
mod persistence;
mod platform;
mod tray;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::Manager;

/// Keep this many recent `gamedockpanel*.log` files in the log dir.
const MAX_SESSION_LOG_FILES: usize = 20;

/// UTC stamp `YYYY-MM-DD_HH-MM-SS` for a unique per-launch log file name.
fn session_log_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil date from Unix days (Howard Hinnant) — no extra crate needed.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let tod = secs % 86_400;
    let h = tod / 3600;
    let min = (tod % 3600) / 60;
    let s = tod % 60;
    format!("{y:04}-{m:02}-{d:02}_{h:02}-{min:02}-{s:02}")
}

fn is_session_log_file(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("gamedockpanel") && n.ends_with(".log"))
}

/// Drop oldest session logs beyond [`MAX_SESSION_LOG_FILES`]. Timestamped
/// per-launch names are not pruned by the plugin's KeepSome (that only
/// rotates within one base name).
fn prune_old_session_logs(log_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, SystemTime)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_session_log_file(p))
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((p, modified))
        })
        .collect();
    if files.len() <= MAX_SESSION_LOG_FILES {
        return;
    }
    files.sort_by_key(|(_, t)| *t);
    let remove_count = files.len() - MAX_SESSION_LOG_FILES;
    for (path, _) in files.into_iter().take(remove_count) {
        if let Err(err) = std::fs::remove_file(&path) {
            log::warn!("prune log {}: {err}", path.display());
        } else {
            log::info!("pruned old log {}", path.display());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let log_file_stem = format!(
        "gamedockpanel-{}-{}",
        session_log_stamp(),
        std::process::id()
    );

    tauri::Builder::default()
        .plugin({
            let builder = tauri_plugin_log::Builder::new();
            #[cfg(debug_assertions)]
            let builder = builder.target(tauri_plugin_log::Target::new(
                tauri_plugin_log::TargetKind::Stdout,
            ));
            builder
                // Debug so [win-backdrop] / geometry details land in release
                // builds too — friend ships logs for remote triage.
                .level(log::LevelFilter::Debug)
                // One file per process launch (not a single forever-append log).
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(
                    MAX_SESSION_LOG_FILES,
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some(log_file_stem.clone()),
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
            commands::diagnostics::log_windows_diag,
            commands::diagnostics::report_webview_render_metrics,
            #[cfg(debug_assertions)]
            commands::settings::qa_set_border,
        ])
        .setup(move |app| {
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
                let log_file = dir.join(format!("{log_file_stem}.log"));
                log::info!("log_dir={}", dir.display());
                log::info!("log_file={}", log_file.display());
                prune_old_session_logs(&dir);
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
