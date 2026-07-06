use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, State};

use crate::platform;

/// Fixed dock roster — carries a bundle ID per entry for real `NSWorkspace`
/// matching. Not user-configurable in this pass.
pub struct AppConfig {
    pub id: &'static str,
    pub name: &'static str,
    pub bundle_id: &'static str,
    pub color: &'static str,
}

pub const APPS: &[AppConfig] = &[
    AppConfig {
        id: "discord",
        name: "Discord",
        bundle_id: "com.hnc.Discord",
        color: "text-indigo-400",
    },
    AppConfig {
        id: "steam",
        name: "Steam",
        bundle_id: "com.valvesoftware.steam",
        color: "text-sky-400",
    },
    AppConfig {
        id: "spotify",
        name: "Spotify",
        bundle_id: "com.spotify.client",
        color: "text-green-400",
    },
    AppConfig {
        id: "obs-studio",
        name: "OBS Studio",
        bundle_id: "com.obsproject.obs-studio",
        color: "text-red-400",
    },
];

/// Wire shape sent to the frontend — must stay in sync with `DockApp` in
/// `src/lib/types.ts`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockAppPayload {
    pub id: String,
    pub name: String,
    pub bundle_id: String,
    pub icon_url: Option<String>,
    pub is_active: bool,
    pub color: String,
}

/// Lightweight running-state update — emitted on launch/terminate without
/// re-sending base64 icon payloads over IPC.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRunningPayload {
    pub id: String,
    pub is_active: bool,
}

/// Icon-only update — emitted at startup and when a late-installed app
/// resolves its native icon.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIconUpdatePayload {
    pub id: String,
    pub icon_url: Option<String>,
}

/// Running state + icon cache, keyed by bundle ID. Deliberately plain data
/// (`bool` / `Option<String>`), not live `NSRunningApplication` handles —
/// see the `tauri-glass-dock` skill for why: AppKit objects aren't `Send`,
/// and Tauri commands run off the main thread where `NSWorkspace`
/// notifications are delivered.
#[derive(Default)]
pub struct AppsState {
    pub running: Mutex<HashMap<&'static str, bool>>,
    pub icons: Mutex<HashMap<&'static str, Option<String>>>,
}

impl AppsState {
    pub fn snapshot(&self) -> Vec<DockAppPayload> {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let icons = self
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        APPS.iter()
            .map(|app| DockAppPayload {
                id: app.id.to_string(),
                name: app.name.to_string(),
                bundle_id: app.bundle_id.to_string(),
                icon_url: icons.get(app.bundle_id).cloned().flatten(),
                is_active: running.get(app.bundle_id).copied().unwrap_or(false),
                color: app.color.to_string(),
            })
            .collect()
    }

    pub fn running_snapshot(&self) -> Vec<AppRunningPayload> {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        APPS.iter()
            .map(|app| AppRunningPayload {
                id: app.id.to_string(),
                is_active: running.get(app.bundle_id).copied().unwrap_or(false),
            })
            .collect()
    }

    pub fn icons_snapshot(&self) -> Vec<AppIconUpdatePayload> {
        let icons = self
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        APPS.iter()
            .map(|app| AppIconUpdatePayload {
                id: app.id.to_string(),
                icon_url: icons.get(app.bundle_id).cloned().flatten(),
            })
            .collect()
    }

    pub fn icon_update_for(&self, bundle_id: &str) -> Option<AppIconUpdatePayload> {
        let config = APPS.iter().find(|app| app.bundle_id == bundle_id)?;
        let icons = self
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        Some(AppIconUpdatePayload {
            id: config.id.to_string(),
            icon_url: icons.get(config.bundle_id).cloned().flatten(),
        })
    }
}

/// One-time pull for the initial render — avoids the race where push events
/// fire before the frontend has subscribed. Running updates arrive via
/// `apps-running-changed`; icon updates via `apps-icons-updated` (see
/// `platform::macos`).
#[tauri::command]
pub fn get_apps_snapshot(state: State<AppsState>) -> Vec<DockAppPayload> {
    state.snapshot()
}

/// Activates the app if a running instance exists (brings its windows to
/// front, does not spawn a second instance); otherwise launches it.
#[tauri::command]
pub fn launch_or_activate_app(app: AppHandle, bundle_id: String) -> Result<(), String> {
    platform::activate_or_launch_app(app, bundle_id)
}
