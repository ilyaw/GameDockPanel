use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, State};

use crate::platform;

/// Fixed dock roster — same 6 apps, order and colors as the original
/// frontend mock (`mockApps.ts`), now carrying a bundle ID for real
/// `NSWorkspace` matching. Not user-configurable in this pass (see
/// PROMPT_04_PROCESS_MONITORING.md — only the data source changes).
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
        id: "minecraft",
        name: "Minecraft",
        // `mdls -name kMDItemCFBundleIdentifier` confirmed on a real install
        // (Homebrew cask `minecraft`) during the stabilization pass —
        // matches the value already here.
        bundle_id: "com.mojang.minecraftlauncher",
        color: "text-lime-400",
    },
    AppConfig {
        id: "obs-studio",
        name: "OBS Studio",
        bundle_id: "com.obsproject.obs-studio",
        color: "text-red-400",
    },
    AppConfig {
        id: "epic-games",
        name: "Epic Games",
        // `mdls -name kMDItemCFBundleIdentifier` confirmed on a real install
        // (Homebrew cask `epic-games`) during the stabilization pass —
        // matches the value already here.
        bundle_id: "com.epicgames.EpicGamesLauncher",
        color: "text-fuchsia-400",
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
}

/// One-time pull for the initial render — avoids the race where
/// `apps-state-changed` fires before the frontend has subscribed to it.
/// Subsequent updates arrive only via that event (see `platform::macos`).
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
