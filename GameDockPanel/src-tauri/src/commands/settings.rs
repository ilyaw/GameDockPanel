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
    /// Tint layer color under the icons (`bg-zinc-950/80`'s replacement).
    /// This is a color layer painted *on top of* the native vibrancy blur,
    /// not a substitute for it — see `tint_opacity` below.
    pub tint_color: String,
    /// Alpha (0.0..=1.0) of the tint layer. At 1.0 the tint is fully
    /// opaque and visually hides the vibrancy blur behind it entirely —
    /// that's an expected consequence of the user's own opacity choice,
    /// not a bug in the vibrancy setup.
    pub tint_opacity: f64,
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
            tint_color: "#09090b".to_string(),
            tint_opacity: 0.8,
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
        match serde_json::from_str::<DockSettings>(&contents) {
            Ok(settings) => return Ok(settings),
            Err(err) => {
                eprintln!(
                    "GameDockPanel: {} is corrupt ({err}), resetting to defaults",
                    path.display()
                );
            }
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
    settings.tint_opacity = settings.tint_opacity.clamp(0.0, 1.0);

    let path = config_file_path(&app)?;
    crate::persistence::write_json_atomic(&path, &settings)?;

    {
        let mut guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = settings.clone();
    }

    let _ = app.emit("dock-settings-changed", settings);
    Ok(())
}
