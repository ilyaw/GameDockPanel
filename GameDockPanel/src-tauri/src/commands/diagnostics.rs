//! Runtime diagnostics for support and debugging.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::apps::AppsState;
use crate::commands::settings::SettingsState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPayload {
    pub platform: String,
    pub arch: String,
    pub app_version: String,
    pub debug_build: bool,
    pub app_data_dir: Option<String>,
    pub log_dir: Option<String>,
    pub dock_apps_path: Option<String>,
    pub dock_settings_path: Option<String>,
    pub app_count: usize,
    pub separator_count: usize,
    pub settings: DiagnosticsSettings,
    pub platform_apps_implemented: bool,
    pub click_through_implemented: bool,
    /// Windows-only Mica / SetWindowRgn snapshot — `null` on other OSes.
    pub windows_backdrop: Option<serde_json::Value>,
    pub recent_log_lines: Vec<String>,
    pub support_hint: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSettings {
    pub dock_position: String,
    pub icon_size_px: f64,
    pub animations_enabled: bool,
    pub border_style: String,
    pub border_width_px: f64,
    pub panel_effect_enabled: bool,
    pub panel_effect: String,
    pub background_animation_enabled: bool,
    pub background_preset: String,
    pub dock_window_layer: String,
    pub static_glow_color: String,
    pub rgb_glow_colors: Vec<String>,
}

#[tauri::command]
pub fn get_diagnostics(app: AppHandle) -> Result<DiagnosticsPayload, String> {
    let platform = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let app_version = app.package_info().version.to_string();
    let debug_build = cfg!(debug_assertions);

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
        let entries = state.entries.lock().map_err(|e| e.to_string())?;
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
        let guard = state.settings.lock().map_err(|e| e.to_string())?;
        DiagnosticsSettings {
            dock_position: format!("{:?}", guard.dock_position),
            icon_size_px: guard.icon_size_px,
            animations_enabled: guard.animations_enabled,
            border_style: guard.border_style.clone(),
            border_width_px: guard.border_width_px,
            panel_effect_enabled: guard.panel_effect_enabled,
            panel_effect: guard.panel_effect.clone(),
            background_animation_enabled: guard.background_animation_enabled,
            background_preset: guard.background_preset.clone(),
            dock_window_layer: format!("{:?}", guard.dock_window_layer),
            static_glow_color: guard.static_glow_color.clone(),
            rgb_glow_colors: guard.rgb_glow_colors.clone(),
        }
    };

    let platform_apps_implemented = cfg!(any(target_os = "macos", target_os = "windows"));
    let click_through_implemented = cfg!(any(target_os = "macos", target_os = "windows"));

    let windows_backdrop = {
        #[cfg(target_os = "windows")]
        {
            app.get_webview_window("main").and_then(|window| {
                serde_json::to_value(crate::platform::windows_backdrop_snapshot(&window)).ok()
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    };

    let recent_log_lines = read_recent_log_lines(&log_dir, 200);

    let support_hint = format!(
        "Send: (1) this JSON from «Скопировать диагностику» (2) the latest gamedockpanel*.log \
         from logDir (3) a screenshot of the dock. Look for lines tagged [win-backdrop] and [dock]."
    );

    log::info!(
        "diagnostics requested: platform={platform}/{arch} apps={app_count} \
         separators={separator_count} debug={debug_build} log_lines={}",
        recent_log_lines.len()
    );
    if let Some(ref snap) = windows_backdrop {
        log::info!("[win-backdrop] diagnostics snapshot={snap}");
    }

    Ok(DiagnosticsPayload {
        platform,
        arch,
        app_version,
        debug_build,
        app_data_dir,
        log_dir,
        dock_apps_path,
        dock_settings_path,
        app_count,
        separator_count,
        settings,
        platform_apps_implemented,
        click_through_implemented,
        windows_backdrop,
        recent_log_lines,
        support_hint,
    })
}

#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    log::info!("open_log_dir: {}", log_dir.display());
    tauri_plugin_opener::open_path(&log_dir, None::<&str>).map_err(|e| e.to_string())
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
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("gamedockpanel") && n.ends_with(".log"))
        })
        .collect();

    // Prefer newest by mtime; fall back to name sort.
    log_files.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });

    let Some(latest) = log_files.last() else {
        return Vec::new();
    };

    log::debug!("diagnostics reading log file={}", latest.display());

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
