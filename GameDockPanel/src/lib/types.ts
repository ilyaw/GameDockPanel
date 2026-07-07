/**
 * A single dock entry. This is the shared contract between the frontend and
 * the Rust process-monitoring commands (`get_apps_snapshot`) that own this
 * data — keep both sides of that boundary compatible with this shape.
 */
export interface DockApp {
  id: string;
  name: string;
  /** macOS bundle identifier — sent back to `launch_or_activate_app` on click. */
  bundleId: string;
  /**
   * Native icon rendered by Rust as a `data:image/png;base64,...` URL, or
   * `null` if it couldn't be resolved (app not installed). Always pair with
   * a fallback — see DockIcon.
   */
  iconUrl: string | null;
  /** Whether the app is currently running, per `NSWorkspace` — not mocked. */
  isActive: boolean;
  /**
   * Tailwind `text-*` class. Sets `currentColor`, reused by the LED both for
   * its fill (`bg-current`) and its glow (`box-shadow: currentColor` in the
   * `led-pulse` keyframes) — one field to keep both in sync.
   */
  color: string;
}

/** Running-state push from Rust — no icon payloads. */
export interface AppRunningUpdate {
  id: string;
  isActive: boolean;
}

/** Icon-only push from Rust — startup batch or late install resolve. */
export interface AppIconUpdate {
  id: string;
  iconUrl: string | null;
}

/**
 * Live-tunable dock visuals — mirrors `DockSettings` in
 * `src-tauri/src/commands/settings.rs`. Read by the dock window to render,
 * read+written by the settings window (`get_dock_settings` /
 * `update_dock_settings`, kept in sync via `dock-settings-changed`).
 */
export interface DockSettings {
  /** Master toggle for decorative-only animation: RGB frame cycle + LED
   * pulse. Does not affect hover-magnify or drag-reorder's layout
   * animation — those are functional interactions, not decoration. */
  animationsEnabled: boolean;
  /** Exactly 6 hex colors — the RGB frame's cycle stops (`--dock-glow-1..6`
   * in `src/index.css`). */
  rgbGlowColors: string[];
  /** Frame color shown when `animationsEnabled` is off, instead of a
   * random freeze-frame of the cycle. */
  staticGlowColor: string;
  /** Tint layer color under the icons — painted on top of the native
   * vibrancy blur, not a substitute for it. */
  tintColor: string;
  /** Alpha (0..1) of the tint layer. */
  tintOpacity: number;
}
