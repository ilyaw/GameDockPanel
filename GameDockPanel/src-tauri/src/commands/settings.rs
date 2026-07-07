use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// Live-tunable visual settings for the dock, persisted to
/// `dock-settings.json` and broadcast to every webview on change (see
/// `update_dock_settings`). Field defaults below mirror the values that
/// used to be hardcoded directly in `src/index.css` / `DockPanel.tsx`
/// before this settings pass existed.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockSettings {
    /// Master toggle for purely decorative animation: the cycling RGB
    /// frame and the LED "breathing" pulse. Does not affect hover-magnify
    /// or drag-reorder's layout animation — those are functional
    /// interactions, not decoration, and are out of scope for this toggle.
    pub animations_enabled: bool,
    /// Exactly 6 hex colors — the RGB frame's cycle stops, read by
    /// `@keyframes rgb-glow` in `src/index.css` via `--dock-glow-1..6`.
    /// Enforced by `update_dock_settings`, not by the type itself (a fixed
    /// `[String; 6]` would work too, but a plain `Vec` keeps this struct
    /// uniform with how the frontend already shapes color arrays, and the
    /// length check lives in one place regardless).
    pub rgb_glow_colors: Vec<String>,
    /// Frame color shown when `animations_enabled` is off — a static
    /// picture, not a random freeze-frame of the cycle.
    pub static_glow_color: String,
    /// Id into the frontend's `BORDER_STYLE_PRESETS` table — same
    /// "id-only, frontend owns the data" pattern as `background_preset`.
    /// Selects which keyframe animation drives the RGB frame: spectrum
    /// cycle, neon pulse, glitch flicker, or a rotating scan ring.
    pub border_style: String,
    /// Master toggle for the panel-body decorative overlay (scanlines /
    /// HUD grid / hologram flicker), independent of `background_animation_enabled`.
    pub panel_effect_enabled: bool,
    /// Id into the frontend's `PANEL_EFFECT_PRESETS` table.
    pub panel_effect: String,
    /// Master toggle for the animated RGB/gradient background layer under
    /// the icons, painted on top of the native vibrancy blur. Independent
    /// of `animations_enabled` (border cycle + LED pulse only) — this is a
    /// separate decorative layer with its own on/off.
    pub background_animation_enabled: bool,
    /// Id into the frontend's `BACKGROUND_PRESETS` table (constants.ts),
    /// which owns the actual color data — this side only stores/persists
    /// the chosen id, same as every other purely-visual string here.
    pub background_preset: String,
    /// 0.0..=1.0 — how vivid/bright the preset's colors render.
    pub background_intensity: f64,
    /// 0.0..=1.0 — opacity of the whole gradient layer over the glass.
    pub background_visibility: f64,
    /// 0.0..=1.0 — flow speed, mapped to an animation duration on the
    /// frontend.
    pub background_speed: f64,
    /// Id into the frontend's `ICON_SIZE_PRESETS` table (constants.ts) —
    /// the single input every dock layout number derives from
    /// (`getSizeMetrics` on the frontend, `size_metrics` in
    /// `platform::macos`). Same "id-only, frontend owns the fallback"
    /// pattern as `background_preset`, except this id is also read on the
    /// Rust side (`icon_size_dip_for_preset`) to compute window geometry
    /// before the DOM has measured anything (startup, add/remove fallback).
    /// `#[serde(default)]` — unlike every other field here, this one is new
    /// in a version that shipped after users could already have a
    /// `dock-settings.json` on disk without it; without a default, that
    /// missing field would fail deserialization of the *entire* file and
    /// silently reset every other already-customized setting back to
    /// defaults too (see `load_or_default_settings`'s corrupt-file path).
    #[serde(default = "default_icon_size_preset")]
    pub icon_size_preset: String,
    /// Continuous icon edge length in logical px (44–72) — the single input
    /// every dock layout number derives from (`getSizeMetrics` on the frontend,
    /// `size_metrics` in `platform::macos`). Read on the Rust side to compute
    /// window geometry before the DOM has measured anything.
    /// `#[serde(default)]` — configs written before this field existed only
    /// had `icon_size_preset`; `load_or_default_settings` back-fills from
    /// that id when `iconSizePx` is absent in the JSON.
    #[serde(default = "default_icon_size_px")]
    pub icon_size_px: f64,
}

fn default_icon_size_preset() -> String {
    "medium".to_string()
}

fn default_icon_size_px() -> f64 {
    56.0
}

const ICON_SIZE_MIN_PX: f64 = 44.0;
const ICON_SIZE_MAX_PX: f64 = 72.0;

fn icon_size_px_from_preset(preset: &str) -> f64 {
    match preset {
        "small" => 44.0,
        "large" => 72.0,
        _ => 56.0,
    }
}

fn clamp_icon_size_px(px: f64) -> f64 {
    px.round().clamp(ICON_SIZE_MIN_PX, ICON_SIZE_MAX_PX)
}

impl Default for DockSettings {
    fn default() -> Self {
        Self {
            animations_enabled: true,
            rgb_glow_colors: vec![
                "#ff3b6b".to_string(),
                "#ff9d3b".to_string(),
                "#e9ff3b".to_string(),
                "#3bffb0".to_string(),
                "#3bb0ff".to_string(),
                "#b03bff".to_string(),
            ],
            static_glow_color: "#ff3b6b".to_string(),
            border_style: "spectrum".to_string(),
            panel_effect_enabled: true,
            panel_effect: "grid".to_string(),
            background_animation_enabled: true,
            background_preset: "chroma".to_string(),
            background_intensity: 0.7,
            background_visibility: 0.45,
            background_speed: 0.4,
            icon_size_preset: "medium".to_string(),
            icon_size_px: 56.0,
        }
    }
}

const RGB_GLOW_COLOR_COUNT: usize = 6;

#[derive(Default)]
pub struct SettingsState {
    pub settings: Mutex<DockSettings>,
}

fn config_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    crate::persistence::app_data_file(app, "dock-settings.json")
}

/// Loads the persisted settings, or falls back to (and immediately
/// persists) `DockSettings::default()` — same first-run/corrupt-file
/// pattern as `commands::apps::load_or_seed_entries`.
fn load_or_default_settings(app: &AppHandle) -> Result<DockSettings, String> {
    let path = config_file_path(app)?;

    if let Ok(contents) = std::fs::read_to_string(&path) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
            if let Ok(mut settings) = serde_json::from_value::<DockSettings>(value.clone()) {
                if value.get("iconSizePx").is_none() {
                    settings.icon_size_px = icon_size_px_from_preset(&settings.icon_size_preset);
                }
                settings.icon_size_px = clamp_icon_size_px(settings.icon_size_px);
                return Ok(settings);
            }
            eprintln!(
                "GameDockPanel: {} is corrupt (invalid dock settings), resetting to defaults",
                path.display()
            );
        } else {
            eprintln!(
                "GameDockPanel: {} is corrupt (invalid JSON), resetting to defaults",
                path.display()
            );
        }
    }

    let defaults = DockSettings::default();
    crate::persistence::write_json_atomic(&path, &defaults)?;
    Ok(defaults)
}

/// Populates `SettingsState` before any window reads it — called once from
/// `lib.rs`'s `.setup()`, alongside `commands::apps::init_entries`.
pub fn init_settings(app: &AppHandle) -> Result<(), String> {
    let settings = load_or_default_settings(app)?;
    let state = app.state::<SettingsState>();
    let mut guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = settings;
    Ok(())
}

/// One-time pull for both the dock and settings windows on mount — pushed
/// updates after that arrive via `dock-settings-changed`.
#[tauri::command]
pub fn get_dock_settings(state: State<SettingsState>) -> DockSettings {
    state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Persists the full settings snapshot and broadcasts it to every webview.
/// Always takes the complete `DockSettings`, not a partial diff — simpler
/// on both ends, and the frontend already holds the full object locally
/// (see `useDockSettings`'s debounced `commit`).
#[tauri::command]
pub fn update_dock_settings(
    app: AppHandle,
    state: State<SettingsState>,
    mut settings: DockSettings,
) -> Result<(), String> {
    if settings.rgb_glow_colors.len() != RGB_GLOW_COLOR_COUNT {
        return Err(format!(
            "rgbGlowColors must have exactly {RGB_GLOW_COLOR_COUNT} entries"
        ));
    }
    settings.background_intensity = settings.background_intensity.clamp(0.0, 1.0);
    settings.background_visibility = settings.background_visibility.clamp(0.0, 1.0);
    settings.background_speed = settings.background_speed.clamp(0.0, 1.0);
    settings.icon_size_px = clamp_icon_size_px(settings.icon_size_px);

    let path = config_file_path(&app)?;
    crate::persistence::write_json_atomic(&path, &settings)?;

    {
        let mut guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = settings.clone();
    }

    // Window geometry during icon-size changes is driven by the dock
    // webview's measured DOM + spring animation (ResizeObserver →
    // `resize_dock_window`), not a formula snap here — an immediate native
    // resize would fight the CSS transition and look like a jump.

    let _ = app.emit("dock-settings-changed", settings);
    Ok(())
}

/// Live icon-size preview while the settings slider is dragged — emits only,
/// no disk write. The dock springs toward this value; `update_dock_settings`
/// persists the final size on release / debounced commit.
#[tauri::command]
pub fn preview_dock_icon_size(app: AppHandle, icon_size_px: f64) -> Result<(), String> {
    let px = clamp_icon_size_px(icon_size_px);
    let _ = app.emit("dock-icon-size-preview", px);
    Ok(())
}
