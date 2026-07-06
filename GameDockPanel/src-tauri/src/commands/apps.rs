use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::platform;

/// One dock entry — persisted to disk and used as the live runtime roster.
/// `id` and `bundle_id` are always equal: a bundle ID is the only stable
/// identifier an arbitrary user-added app has, so there's no value in also
/// maintaining a synthetic human slug (the original curated roster used
/// slugs like `"discord"`; dropped in favor of this simpler invariant).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub bundle_id: String,
    pub color: String,
}

/// Curated first-run candidate — `'static` compile-time data, never
/// persisted itself (only copied into an owned `AppEntry` once resolved as
/// actually installed on disk).
struct SeedCandidate {
    name: &'static str,
    bundle_id: &'static str,
    color: &'static str,
}

/// First-run candidate pool, ordered by priority. Only entries that resolve
/// as installed (via the same bundle-id resolve used everywhere else) seed
/// the dock, and only the first `DEFAULT_SEED_LIMIT` of those — see there
/// for why order matters. Expandable: add more candidates here as the
/// product grows, without touching the seeding logic itself. Deliberately
/// gaming/streaming-adjacent only (see PROMPT_06_CUSTOM_APPS.md) — Chrome
/// and general-purpose apps are a separate product decision, not added here.
const SEED_POOL: &[SeedCandidate] = &[
    SeedCandidate {
        name: "Discord",
        bundle_id: "com.hnc.Discord",
        color: "text-indigo-400",
    },
    SeedCandidate {
        name: "Steam",
        bundle_id: "com.valvesoftware.steam",
        color: "text-sky-400",
    },
    SeedCandidate {
        name: "Spotify",
        bundle_id: "com.spotify.client",
        color: "text-green-400",
    },
    SeedCandidate {
        name: "OBS Studio",
        bundle_id: "com.obsproject.obs-studio",
        color: "text-red-400",
    },
    SeedCandidate {
        name: "Epic Games",
        // Confirmed via `mdls` during the process-monitoring pass (see
        // CLAUDE.md / commit history) before Epic was later trimmed from
        // the then-static roster. Reused here unchanged.
        bundle_id: "com.epicgames.EpicGamesLauncher",
        color: "text-fuchsia-400",
    },
    SeedCandidate {
        name: "Battle.net",
        // Web-verified only (Homebrew cask uninstall path, macupdater.com,
        // `net.battle.*` preference file prefixes) — not confirmed with
        // `mdls` on this machine. Same caveat category as Epic/Minecraft
        // before their local verification; re-check with `mdls -name
        // kMDItemCFBundleIdentifier` if this ever shows a wrong state.
        bundle_id: "net.battle.app",
        color: "text-blue-400",
    },
];

/// Caps how many `SEED_POOL` candidates seed a first-run dock. Independent
/// of `MAX_APPS`: this only bounds *automatic* first-run population, not
/// what a user can add by hand afterwards. Keeps a dock from opening fully
/// packed on a machine with many gaming launchers installed, without any
/// user action — see PROMPT_06_CUSTOM_APPS.md follow-up question.
const DEFAULT_SEED_LIMIT: usize = 10;

/// Soft ceiling on total dock entries (seeded + manually added). Purely a
/// pill-width safety net now that width resizes dynamically — not for
/// reserving native window space. The "+" tile disables past this rather
/// than disappearing (still visible so the limit is discoverable).
pub const MAX_APPS: usize = 15;

/// Small, fixed palette for arbitrary user-added apps — colors are picked
/// deterministically from a hash of the bundle ID, not sampled from icon
/// pixels (a harder, separate task out of scope for this pass). Disjoint
/// from every `SEED_POOL` color so a manually added app never visually
/// collides with a seeded one.
const COLOR_PALETTE: &[&str] = &[
    "text-orange-400",
    "text-purple-400",
    "text-pink-400",
    "text-yellow-400",
    "text-teal-400",
    "text-rose-400",
    "text-lime-400",
    "text-violet-400",
];

fn color_for_bundle_id(bundle_id: &str) -> &'static str {
    let mut hasher = DefaultHasher::new();
    bundle_id.hash(&mut hasher);
    let index = (hasher.finish() as usize) % COLOR_PALETTE.len();
    COLOR_PALETTE[index]
}

/// Derives a display name from a `.app` bundle's filename (e.g.
/// `/Applications/Discord.app` → `"Discord"`) rather than reading
/// `CFBundleName` out of `Info.plist` — the filename is what the user just
/// picked in the native dialog, and is enough for this pass.
fn display_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
        .to_string()
}

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
///
/// `entries` is the persisted, user-configurable roster (order = dock
/// order) — the single source of truth `running`/`icons` are keyed against.
/// `pill_width_dip` mirrors the last width applied by
/// `platform::sync_dock_geometry`; hit-testing reads it directly instead of
/// a compile-time constant now that width changes at runtime. It lives here
/// rather than in a separate managed state because it's derived straight
/// from `entries.len()` and only ever changes alongside it.
#[derive(Default)]
pub struct AppsState {
    pub entries: Mutex<Vec<AppEntry>>,
    pub running: Mutex<HashMap<String, bool>>,
    pub icons: Mutex<HashMap<String, Option<String>>>,
    pub pill_width_dip: Mutex<f64>,
}

impl AppsState {
    pub fn snapshot(&self) -> Vec<DockAppPayload> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let icons = self
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        entries
            .iter()
            .map(|app| DockAppPayload {
                id: app.id.clone(),
                name: app.name.clone(),
                bundle_id: app.bundle_id.clone(),
                icon_url: icons.get(&app.bundle_id).cloned().flatten(),
                is_active: running.get(&app.bundle_id).copied().unwrap_or(false),
                color: app.color.clone(),
            })
            .collect()
    }

    pub fn running_snapshot(&self) -> Vec<AppRunningPayload> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        entries
            .iter()
            .map(|app| AppRunningPayload {
                id: app.id.clone(),
                is_active: running.get(&app.bundle_id).copied().unwrap_or(false),
            })
            .collect()
    }

    pub fn icons_snapshot(&self) -> Vec<AppIconUpdatePayload> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let icons = self
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        entries
            .iter()
            .map(|app| AppIconUpdatePayload {
                id: app.id.clone(),
                icon_url: icons.get(&app.bundle_id).cloned().flatten(),
            })
            .collect()
    }

    pub fn icon_update_for(&self, bundle_id: &str) -> Option<AppIconUpdatePayload> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = entries.iter().find(|app| app.bundle_id == bundle_id)?;
        let id = entry.id.clone();
        drop(entries);

        let icons = self
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        Some(AppIconUpdatePayload {
            id,
            icon_url: icons.get(bundle_id).cloned().flatten(),
        })
    }

    pub fn app_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

fn config_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("dock-apps.json"))
}

/// Writes `entries` to disk atomically: a crash or force-quit mid-write
/// leaves the previous, still-valid file in place instead of a truncated
/// one — write to a sibling `.tmp` file, then `rename` (atomic on the same
/// filesystem) over the real path.
fn save_entries(app: &AppHandle, entries: &[AppEntry]) -> Result<(), String> {
    let path = config_file_path(app)?;
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    std::fs::write(&tmp_path, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Builds the first-run dock from `SEED_POOL`: walks it in priority order,
/// keeps only candidates that resolve as actually installed, and stops
/// once `DEFAULT_SEED_LIMIT` matches are found. Does not exist to prune an
/// already-persisted list — this only ever runs once, when no config file
/// exists yet.
fn seed_entries() -> Vec<AppEntry> {
    SEED_POOL
        .iter()
        .filter(|candidate| platform::is_app_installed(candidate.bundle_id))
        .take(DEFAULT_SEED_LIMIT)
        .map(|candidate| AppEntry {
            id: candidate.bundle_id.to_string(),
            name: candidate.name.to_string(),
            bundle_id: candidate.bundle_id.to_string(),
            color: candidate.color.to_string(),
        })
        .collect()
}

/// Loads the persisted roster, or — on first run, when no config file
/// exists — computes and immediately persists the installed-candidate
/// seed. Persisting the seed right away (rather than waiting for the first
/// user edit) means a machine with zero matching candidates stays an empty
/// dock on every subsequent launch, instead of re-running the installed
/// check every time a config file happens to be missing.
fn load_or_seed_entries(app: &AppHandle) -> Result<Vec<AppEntry>, String> {
    let path = config_file_path(app)?;

    if let Ok(contents) = std::fs::read_to_string(&path) {
        match serde_json::from_str::<Vec<AppEntry>>(&contents) {
            Ok(entries) => return Ok(entries),
            Err(err) => {
                eprintln!(
                    "GameDockPanel: {} is corrupt ({err}), reseeding from candidates",
                    path.display()
                );
            }
        }
    }

    let seeded = seed_entries();
    save_entries(app, &seeded)?;
    Ok(seeded)
}

/// Populates `AppsState.entries` before the window/monitoring are set up —
/// called once from `lib.rs`'s `.setup()`, ahead of `setup_dock_window` so
/// the initial window size is computed from the real entry count instead
/// of a placeholder.
pub fn init_entries(app: &AppHandle) -> Result<(), String> {
    let entries = load_or_seed_entries(app)?;
    let state = app.state::<AppsState>();
    let mut guard = state
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = entries;
    Ok(())
}

/// One-time pull for the initial render — avoids the race where push events
/// fire before the frontend has subscribed. Running updates arrive via
/// `apps-running-changed`; icon updates via `apps-icons-updated`; list
/// membership changes (add/remove) via `apps-list-changed` (see
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

/// Resolves the bundle ID from a user-picked `.app` path (native `Open`
/// dialog, filtered to `/Applications`), adds it to the dock, resolves its
/// icon and current running state immediately (the app is confirmed
/// installed at this point — no late-resolve needed on this path), persists
/// the updated roster, and resizes the window. Emits `apps-list-changed`
/// rather than returning the new list directly, matching the event-driven
/// pattern already used for running/icon updates.
#[tauri::command]
pub fn add_app_from_path(app: AppHandle, state: State<AppsState>, path: String) -> Result<(), String> {
    let bundle_id = platform::resolve_bundle_id_from_path(&path)?;

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.iter().any(|entry| entry.bundle_id == bundle_id) {
            return Err(format!("{} is already in the dock", bundle_id));
        }
        if entries.len() >= MAX_APPS {
            return Err("dock is full".to_string());
        }
    }

    let entry = AppEntry {
        id: bundle_id.clone(),
        name: display_name_from_path(&path),
        bundle_id: bundle_id.clone(),
        color: color_for_bundle_id(&bundle_id).to_string(),
    };

    let app_count = {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.push(entry);
        entries.len()
    };

    {
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        running.insert(bundle_id.clone(), platform::is_bundle_running(&bundle_id));
    }
    {
        let mut icons = state
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        icons.insert(bundle_id.clone(), platform::resolve_app_icon(&bundle_id));
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        save_entries(&app, &entries)?;
    }

    if let Some(window) = app.get_webview_window("main") {
        platform::sync_dock_geometry(&window, app_count)?;
    }

    let _ = app.emit("apps-list-changed", state.snapshot());
    Ok(())
}

/// Removes an entry from the persisted list and `AppsState` only — never
/// terminates the app's own running process, this only affects what the
/// dock displays.
#[tauri::command]
pub fn remove_app(app: AppHandle, state: State<AppsState>, bundle_id: String) -> Result<(), String> {
    let app_count = {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = entries.len();
        entries.retain(|entry| entry.bundle_id != bundle_id);
        if entries.len() == before {
            return Err(format!("{} is not in the dock", bundle_id));
        }
        entries.len()
    };

    {
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        running.remove(&bundle_id);
    }
    {
        let mut icons = state
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        icons.remove(&bundle_id);
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        save_entries(&app, &entries)?;
    }

    if let Some(window) = app.get_webview_window("main") {
        platform::sync_dock_geometry(&window, app_count)?;
    }

    let _ = app.emit("apps-list-changed", state.snapshot());
    Ok(())
}

/// Reorders the persisted roster to match `ordered_bundle_ids` — same set of
/// IDs, new sequence. Emits `apps-list-changed`; does not resize (count
/// unchanged).
#[tauri::command]
pub fn reorder_apps(
    app: AppHandle,
    state: State<AppsState>,
    ordered_bundle_ids: Vec<String>,
) -> Result<(), String> {
    let snapshot = {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let current: HashSet<&str> = entries.iter().map(|e| e.bundle_id.as_str()).collect();
        let proposed: HashSet<&str> = ordered_bundle_ids.iter().map(String::as_str).collect();
        if current != proposed {
            return Err("reorder list does not match current dock entries".to_string());
        }
        if ordered_bundle_ids.len() != entries.len() {
            return Err("reorder list length mismatch".to_string());
        }

        let by_id: HashMap<String, AppEntry> = entries
            .iter()
            .map(|entry| (entry.bundle_id.clone(), entry.clone()))
            .collect();
        let reordered = ordered_bundle_ids
            .iter()
            .map(|bundle_id| {
                by_id
                    .get(bundle_id)
                    .cloned()
                    .ok_or_else(|| format!("{bundle_id} is not in the dock"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        *entries = reordered;
        entries.clone()
    };

    save_entries(&app, &snapshot)?;
    let _ = app.emit("apps-list-changed", state.snapshot());
    Ok(())
}
