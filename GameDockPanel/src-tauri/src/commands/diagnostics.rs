//! Runtime diagnostics for support and debugging.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::apps::AppsState;
use crate::commands::settings::SettingsState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPayload {
    pub platform: String,
    pub app_version: String,
    pub app_data_dir: Option<String>,
    pub log_dir: Option<String>,
    pub dock_apps_path: Option<String>,
    pub dock_settings_path: Option<String>,
    pub app_count: usize,
    pub separator_count: usize,
    pub settings: DiagnosticsSettings,
    pub platform_apps_implemented: bool,
    pub click_through_implemented: bool,
    pub recent_log_lines: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSettings {
    pub dock_position: String,
    pub icon_size_px: f64,
    pub animations_enabled: bool,
    pub panel_effect: String,
    pub dock_window_layer: String,
}

#[tauri::command]
pub fn get_diagnostics(app: AppHandle) -> Result<DiagnosticsPayload, String> {
    let platform = std::env::consts::OS.to_string();
    let app_version = app.package_info().version.to_string();

    let app_data_dir = app.path().app_data_dir().ok().map(|p| p.display().to_string());
    let log_dir = app.path().app_log_dir().ok().map(|p| p.display().to_string());
    let dock_apps_path = crate::persistence::app_data_file(&app, "dock-apps.json")
        .ok()
        .map(|p| p.display().to_string());
    let dock_settings_path = crate::persistence::app_data_file(&app, "dock-settings.json")
        .ok()
        .map(|p| p.display().to_string());

    let (app_count, separator_count) = {
        let state = app.state::<AppsState>();
        let entries = state
            .entries
            .lock()
            .map_err(|e| e.to_string())?;
        let apps = entries
            .iter()
            .filter(|item| matches!(item, crate::commands::apps::DockItem::App(_)))
            .count();
        let separators = entries
            .iter()
            .filter(|item| matches!(item, crate::commands::apps::DockItem::Separator { .. }))
            .count();
        (apps, separators)
    };

    let settings = {
        let state = app.state::<SettingsState>();
        let guard = state
            .settings
            .lock()
            .map_err(|e| e.to_string())?;
        DiagnosticsSettings {
            dock_position: format!("{:?}", guard.dock_position),
            icon_size_px: guard.icon_size_px,
            animations_enabled: guard.animations_enabled,
            panel_effect: guard.panel_effect.clone(),
            dock_window_layer: format!("{:?}", guard.dock_window_layer),
        }
    };

    let platform_apps_implemented = cfg!(any(target_os = "macos", target_os = "windows"));
    let click_through_implemented = cfg!(any(target_os = "macos", target_os = "windows"));

    let recent_log_lines = read_recent_log_lines(&log_dir, 40);

    log::info!(
        "diagnostics requested: platform={platform} apps={app_count} separators={separator_count}"
    );

    Ok(DiagnosticsPayload {
        platform,
        app_version,
        app_data_dir,
        log_dir,
        dock_apps_path,
        dock_settings_path,
        app_count,
        separator_count,
        settings,
        platform_apps_implemented,
        click_through_implemented,
        recent_log_lines,
    })
}

#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    tauri_plugin_opener::open_path(&log_dir, None::<&str>)
        .map_err(|e| e.to_string())
}

fn read_recent_log_lines(log_dir: &Option<String>, max_lines: usize) -> Vec<String> {
    let Some(dir) = log_dir else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut log_files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    log_files.sort();

    let Some(latest) = log_files.last() else {
        return Vec::new();
    };

    let Ok(contents) = std::fs::read_to_string(latest) else {
        return Vec::new();
    };

    contents
        .lines()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(String::from)
        .collect()
}
