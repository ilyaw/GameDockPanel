use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::settings::{DockSettings, SettingsState};
use crate::platform::{self, IconResolveResult};

fn icon_metrics_for_app(app: &AppHandle) -> (f64, f64) {
    let icon_size_dip = {
        let state = app.state::<SettingsState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.icon_size_px
    };
    let scale_factor = app
        .get_webview_window("main")
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(2.0);
    (icon_size_dip, scale_factor)
}

/// Neutral LED fallback when `ledColorMode` is `override_only` and no manual
/// color is set for an app.
pub const LED_NEUTRAL_FALLBACK: &str = "#a1a1aa";

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
    /// Manual LED hex override (`#rrggbb`). `None` uses auto-sampled color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_override: Option<String>,
    /// Legacy Tailwind `text-*` class from configs before icon sampling.
    /// Ignored on read except when it is already a hex override.
    #[serde(default, skip_serializing)]
    color: Option<String>,
}

impl AppEntry {
    fn normalize_legacy_color(mut self) -> Self {
        if self.color_override.is_none() {
            if let Some(legacy) = self.color.take() {
                if legacy.starts_with('#') {
                    self.color_override = Some(legacy);
                }
            }
        }
        self
    }
}

/// A visual divider between dock app groups — persisted, no bundle ID or icon.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeparatorEntry {
    pub id: String,
}

/// One persisted dock row — either an app or a separator.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DockItem {
    #[serde(rename = "app")]
    App(AppEntry),
    #[serde(rename = "separator")]
    Separator(SeparatorEntry),
}

impl DockItem {
    pub fn id(&self) -> &str {
        match self {
            DockItem::App(entry) => &entry.id,
            DockItem::Separator(separator) => &separator.id,
        }
    }

    pub fn app_entry(&self) -> Option<&AppEntry> {
        match self {
            DockItem::App(entry) => Some(entry),
            DockItem::Separator(_) => None,
        }
    }
}

pub fn count_apps(entries: &[DockItem]) -> usize {
    entries
        .iter()
        .filter(|item| matches!(item, DockItem::App(_)))
        .count()
}

pub fn count_separators(entries: &[DockItem]) -> usize {
    entries
        .iter()
        .filter(|item| matches!(item, DockItem::Separator(_)))
        .count()
}

/// Curated first-run candidate — `'static` compile-time data, never
/// persisted itself (only copied into an owned `AppEntry` once resolved as
/// actually installed on disk).
struct SeedCandidate {
    name: &'static str,
    bundle_id: &'static str,
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
    },
    SeedCandidate {
        name: "Steam",
        bundle_id: "com.valvesoftware.steam",
    },
    SeedCandidate {
        name: "Spotify",
        bundle_id: "com.spotify.client",
    },
    SeedCandidate {
        name: "OBS Studio",
        bundle_id: "com.obsproject.obs-studio",
    },
    SeedCandidate {
        name: "Epic Games",
        bundle_id: "com.epicgames.EpicGamesLauncher",
    },
    SeedCandidate {
        name: "Battle.net",
        bundle_id: "net.battle.app",
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
/// reserving native window space. Drag-drop past this limit is rejected
/// with the reject-pulse cue (see `add_app_from_path` below).
pub const MAX_APPS: usize = 15;

/// Soft ceiling on separator count — keeps the pill from degrading into an
/// endless row of empty lines.
pub const MAX_SEPARATORS: usize = 5;

/// Deterministic hex fallback when icon sampling fails — keyed by bundle ID.
const COLOR_PALETTE: &[&str] = &[
    "#fb923c",
    "#c084fc",
    "#f472b6",
    "#facc15",
    "#2dd4bf",
    "#fb7185",
    "#a3e635",
    "#a78bfa",
];

fn fallback_hex_for_bundle_id(bundle_id: &str) -> &'static str {
    let mut hasher = DefaultHasher::new();
    bundle_id.hash(&mut hasher);
    let index = (hasher.finish() as usize) % COLOR_PALETTE.len();
    COLOR_PALETTE[index]
}

fn is_valid_hex_color(color: &str) -> bool {
    let color = color.trim();
    if color.len() != 7 || !color.starts_with('#') {
        return false;
    }
    color[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Derives a display name from a `.app` bundle's filename (e.g.
/// `/Applications/Discord.app` → `"Discord"`) rather than reading
/// `CFBundleName` out of `Info.plist` — the filename is what the user just
/// dropped onto the dock, and is enough for this pass.
fn display_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
        .to_string()
}

/// App fields inside a dock snapshot — must stay in sync with `DockApp` in
/// `src/lib/types.ts`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockAppPayload {
    pub id: String,
    pub name: String,
    pub bundle_id: String,
    pub icon_url: Option<String>,
    pub is_active: bool,
    pub indicator_color: String,
    pub indicator_color_auto: String,
    pub indicator_color_override: Option<String>,
}

/// Wire shape for one dock row — tagged union mirroring `DockItem` /
/// `DockItem` in `src/lib/types.ts`.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DockItemPayload {
    #[serde(rename = "app")]
    App {
        #[serde(flatten)]
        app: DockAppPayload,
    },
    #[serde(rename = "separator")]
    Separator { id: String },
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
    pub entries: Mutex<Vec<DockItem>>,
    pub running: Mutex<HashMap<String, bool>>,
    pub icons: Mutex<HashMap<String, Option<String>>>,
    pub auto_colors: Mutex<HashMap<String, String>>,
    pub pill_width_dip: Mutex<f64>,
    pub pill_height_dip: Mutex<f64>,
    pub menu_overlay_height_dip: Mutex<f64>,
}

pub(crate) fn resolve_indicator_color(
    settings: &DockSettings,
    entry: &AppEntry,
    auto_colors: &HashMap<String, String>,
) -> (String, String, Option<String>) {
    let auto = auto_colors
        .get(&entry.bundle_id)
        .cloned()
        .unwrap_or_else(|| fallback_hex_for_bundle_id(&entry.bundle_id).to_string());
    let indicator = match settings.led_color_mode.as_str() {
        "fixed" => settings.led_fixed_color.clone(),
        "override_only" => entry
            .color_override
            .clone()
            .unwrap_or_else(|| LED_NEUTRAL_FALLBACK.to_string()),
        _ => entry
            .color_override
            .clone()
            .unwrap_or_else(|| auto.clone()),
    };
    (indicator, auto, entry.color_override.clone())
}

pub(crate) fn apply_icon_resolve(state: &AppsState, bundle_id: &str, resolved: IconResolveResult) {
    {
        let mut icons = state
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        icons.insert(bundle_id.to_string(), resolved.icon_url);
    }
    if let Some(color) = resolved.accent_color {
        let mut auto_colors = state
            .auto_colors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        auto_colors.insert(bundle_id.to_string(), color);
    }
}

impl AppsState {
    pub fn snapshot(&self, settings: &DockSettings) -> Vec<DockItemPayload> {
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
        let auto_colors = self
            .auto_colors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        entries
            .iter()
            .map(|item| match item {
                DockItem::App(app) => {
                    let (indicator_color, indicator_color_auto, indicator_color_override) =
                        resolve_indicator_color(settings, app, &auto_colors);
                    DockItemPayload::App {
                        app: DockAppPayload {
                            id: app.id.clone(),
                            name: app.name.clone(),
                            bundle_id: app.bundle_id.clone(),
                            icon_url: icons.get(&app.bundle_id).cloned().flatten(),
                            is_active: running.get(&app.bundle_id).copied().unwrap_or(false),
                            indicator_color,
                            indicator_color_auto,
                            indicator_color_override,
                        },
                    }
                }
                DockItem::Separator(separator) => DockItemPayload::Separator {
                    id: separator.id.clone(),
                },
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
            .filter_map(|item| {
                let DockItem::App(app) = item else {
                    return None;
                };
                Some(AppRunningPayload {
                    id: app.id.clone(),
                    is_active: running.get(&app.bundle_id).copied().unwrap_or(false),
                })
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
            .filter_map(|item| {
                let DockItem::App(app) = item else {
                    return None;
                };
                Some(AppIconUpdatePayload {
                    id: app.id.clone(),
                    icon_url: icons.get(&app.bundle_id).cloned().flatten(),
                })
            })
            .collect()
    }

    pub fn icon_update_for(&self, bundle_id: &str) -> Option<AppIconUpdatePayload> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = entries.iter().find_map(|item| match item {
            DockItem::App(app) if app.bundle_id == bundle_id => Some(app),
            _ => None,
        })?;
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

    pub fn entries_snapshot(&self) -> Vec<DockItem> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn config_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    crate::persistence::app_data_file(app, "dock-apps.json")
}

/// Writes `entries` to disk atomically — see `persistence::write_json_atomic`.
fn save_entries(app: &AppHandle, entries: &[DockItem]) -> Result<(), String> {
    let path = config_file_path(app)?;
    crate::persistence::write_json_atomic(&path, &entries)
}

fn seed_entries() -> Vec<DockItem> {
    SEED_POOL
        .iter()
        .filter(|candidate| platform::is_app_installed(candidate.bundle_id))
        .take(DEFAULT_SEED_LIMIT)
        .map(|candidate| {
            DockItem::App(AppEntry {
                id: candidate.bundle_id.to_string(),
                name: candidate.name.to_string(),
                bundle_id: candidate.bundle_id.to_string(),
                color_override: None,
                color: None,
            })
        })
        .collect()
}

fn parse_dock_items_json(contents: &str) -> Result<Vec<DockItem>, String> {
    let values: Vec<serde_json::Value> =
        serde_json::from_str(contents).map_err(|err| err.to_string())?;

    values
        .into_iter()
        .map(|value| {
            if value.get("type").is_none() {
                let entry: AppEntry =
                    serde_json::from_value(value).map_err(|err| err.to_string())?;
                Ok(DockItem::App(entry.normalize_legacy_color()))
            } else {
                serde_json::from_value(value).map_err(|err| err.to_string())
            }
        })
        .collect()
}

fn load_or_seed_entries(app: &AppHandle) -> Result<Vec<DockItem>, String> {
    let path = config_file_path(app)?;

    if let Ok(contents) = std::fs::read_to_string(&path) {
        match parse_dock_items_json(&contents) {
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

fn settings_snapshot(app: &AppHandle) -> DockSettings {
    app.state::<SettingsState>()
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub(crate) fn emit_apps_list_changed(app: &AppHandle, state: &AppsState) {
    let settings = settings_snapshot(app);
    let _ = app.emit("apps-list-changed", state.snapshot(&settings));
}

#[tauri::command]
pub fn get_apps_snapshot(app: AppHandle, state: State<AppsState>) -> Vec<DockItemPayload> {
    let settings = settings_snapshot(&app);
    state.snapshot(&settings)
}

#[tauri::command]
pub fn launch_or_activate_app(app: AppHandle, bundle_id: String) -> Result<(), String> {
    platform::activate_or_launch_app(app, bundle_id)
}

#[tauri::command]
pub fn add_app_from_path(
    app: AppHandle,
    state: State<AppsState>,
    path: String,
    insert_index: Option<usize>,
) -> Result<(), String> {
    let bundle_id = platform::resolve_bundle_id_from_path(&path)?;

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.iter().any(|item| {
            item.app_entry()
                .is_some_and(|entry| entry.bundle_id == bundle_id)
        }) {
            return Err(format!("{} is already in the dock", bundle_id));
        }
        if count_apps(&entries) >= MAX_APPS {
            return Err("dock is full".to_string());
        }
    }

    let entry = AppEntry {
        id: bundle_id.clone(),
        name: display_name_from_path(&path),
        bundle_id: bundle_id.clone(),
        color_override: None,
        color: None,
    };

    {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let idx = insert_index.unwrap_or(entries.len()).min(entries.len());
        entries.insert(idx, DockItem::App(entry));
    }

    {
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        running.insert(bundle_id.clone(), platform::is_bundle_running(&bundle_id));
    }
    {
        let (icon_size_dip, scale_factor) = icon_metrics_for_app(&app);
        let resolved = platform::resolve_app_icon(&bundle_id, icon_size_dip, scale_factor);
        apply_icon_resolve(&state, &bundle_id, resolved);
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        save_entries(&app, &entries)?;
    }

    emit_apps_list_changed(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn remove_app(app: AppHandle, state: State<AppsState>, bundle_id: String) -> Result<(), String> {
    let removed = {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = entries.len();
        entries.retain(|item| {
            !matches!(
                item,
                DockItem::App(entry) if entry.bundle_id == bundle_id
            )
        });
        entries.len() != before
    };
    if !removed {
        return Err(format!("{} is not in the dock", bundle_id));
    }

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
        let mut auto_colors = state
            .auto_colors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        auto_colors.remove(&bundle_id);
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        save_entries(&app, &entries)?;
    }

    emit_apps_list_changed(&app, &state);
    Ok(())
}

pub(crate) fn refresh_icon_cache(app: &AppHandle, state: &AppsState) {
    platform::refresh_dock_icons(app, state);
}

/// Re-samples accent colors from native icons without changing icon size.
#[tauri::command]
pub fn refresh_indicator_colors(app: AppHandle, state: State<AppsState>) -> Result<(), String> {
    let entries = state
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let (icon_size_dip, scale_factor) = icon_metrics_for_app(&app);

    for item in entries.iter() {
        let DockItem::App(entry) = item else {
            continue;
        };
        let resolved = platform::resolve_app_icon(&entry.bundle_id, icon_size_dip, scale_factor);
        apply_icon_resolve(&state, &entry.bundle_id, resolved);
    }

    emit_apps_list_changed(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn set_app_indicator_color(
    app: AppHandle,
    state: State<AppsState>,
    bundle_id: String,
    color: Option<String>,
) -> Result<(), String> {
    if let Some(ref hex) = color {
        if !is_valid_hex_color(hex) {
            return Err("color must be a #rrggbb hex value".to_string());
        }
    }

    {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(entry) = entries.iter_mut().find_map(|item| match item {
            DockItem::App(app) if app.bundle_id == bundle_id => Some(app),
            _ => None,
        }) else {
            return Err(format!("{bundle_id} is not in the dock"));
        };
        entry.color_override = color;
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        save_entries(&app, &entries)?;
    }

    emit_apps_list_changed(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn reveal_app_in_finder(app: AppHandle, bundle_id: String) -> Result<(), String> {
    platform::reveal_app_in_finder(app, bundle_id)
}

#[tauri::command]
pub fn quit_app(app: AppHandle, bundle_id: String) -> Result<(), String> {
    platform::quit_app(app, bundle_id)
}

#[tauri::command]
pub fn reorder_apps(
    app: AppHandle,
    state: State<AppsState>,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    let snapshot = {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let current: HashSet<&str> = entries.iter().map(|item| item.id()).collect();
        let proposed: HashSet<&str> = ordered_ids.iter().map(String::as_str).collect();
        if current != proposed {
            return Err("reorder list does not match current dock entries".to_string());
        }
        if ordered_ids.len() != entries.len() {
            return Err("reorder list length mismatch".to_string());
        }

        let by_id: HashMap<String, DockItem> = entries
            .iter()
            .map(|item| (item.id().to_string(), item.clone()))
            .collect();
        let reordered = ordered_ids
            .iter()
            .map(|id| {
                by_id
                    .get(id)
                    .cloned()
                    .ok_or_else(|| format!("{id} is not in the dock"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        *entries = reordered;
        entries.clone()
    };

    save_entries(&app, &snapshot)?;
    emit_apps_list_changed(&app, &state);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SeparatorPlacement {
    Before,
    After,
}

#[tauri::command]
pub fn insert_separator(
    app: AppHandle,
    state: State<AppsState>,
    bundle_id: String,
    placement: SeparatorPlacement,
    separator_id: String,
) -> Result<(), String> {
    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if count_separators(&entries) >= MAX_SEPARATORS {
            return Err("separator limit reached".to_string());
        }
        if entries.iter().any(|item| item.id() == separator_id) {
            return Err("separator id already exists".to_string());
        }
    }

    {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let app_index = entries.iter().position(|item| {
            matches!(
                item,
                DockItem::App(entry) if entry.bundle_id == bundle_id
            )
        });
        let Some(app_index) = app_index else {
            return Err(format!("{bundle_id} is not in the dock"));
        };

        let insert_index = match placement {
            SeparatorPlacement::Before => app_index,
            SeparatorPlacement::After => app_index + 1,
        };

        if insert_index < entries.len()
            && matches!(entries[insert_index], DockItem::Separator(_))
        {
            return Err("separator already at this position".to_string());
        }
        if insert_index > 0
            && matches!(entries[insert_index - 1], DockItem::Separator(_))
        {
            return Err("adjacent separator not allowed".to_string());
        }

        entries.insert(
            insert_index,
            DockItem::Separator(SeparatorEntry { id: separator_id }),
        );
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        save_entries(&app, &entries)?;
    }

    emit_apps_list_changed(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn remove_separator(
    app: AppHandle,
    state: State<AppsState>,
    separator_id: String,
) -> Result<(), String> {
    let removed = {
        let mut entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = entries.len();
        entries.retain(|item| {
            !matches!(
                item,
                DockItem::Separator(separator) if separator.id == separator_id
            )
        });
        entries.len() != before
    };
    if !removed {
        return Err(format!("{separator_id} is not in the dock"));
    }

    {
        let entries = state
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        save_entries(&app, &entries)?;
    }

    emit_apps_list_changed(&app, &state);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_legacy_flat_app_entries_without_type() {
        let json = r#"[{"id":"com.hnc.Discord","name":"Discord","bundleId":"com.hnc.Discord"}]"#;
        let items = parse_dock_items_json(json).expect("legacy json should parse");
        assert_eq!(items.len(), 1);
        match &items[0] {
            DockItem::App(entry) => {
                assert_eq!(entry.bundle_id, "com.hnc.Discord");
                assert_eq!(entry.id, "com.hnc.Discord");
                assert_eq!(entry.name, "Discord");
            }
            DockItem::Separator(_) => panic!("expected app entry"),
        }
    }

    #[test]
    fn load_tagged_dock_items_with_separator() {
        let json = r#"[
            {"type":"app","id":"com.hnc.Discord","name":"Discord","bundleId":"com.hnc.Discord"},
            {"type":"separator","id":"sep-1"}
        ]"#;
        let items = parse_dock_items_json(json).expect("tagged json should parse");
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], DockItem::App(_)));
        assert!(matches!(&items[1], DockItem::Separator(_)));
    }

    #[test]
    fn snapshot_payload_flattens_app_fields_for_frontend() {
        let payload = DockItemPayload::App {
            app: DockAppPayload {
                id: "com.test.app".to_string(),
                name: "Test".to_string(),
                bundle_id: "com.test.app".to_string(),
                icon_url: None,
                is_active: false,
                indicator_color: "#ffffff".to_string(),
                indicator_color_auto: "#ffffff".to_string(),
                indicator_color_override: None,
            },
        };
        let json = serde_json::to_value(payload).expect("serialize snapshot");
        assert_eq!(json.get("type").and_then(|v| v.as_str()), Some("app"));
        assert_eq!(
            json.get("bundleId").and_then(|v| v.as_str()),
            Some("com.test.app")
        );
        assert!(json.get("app").is_none());
    }
}
