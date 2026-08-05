/**
 * A single dock entry. This is the shared contract between the frontend and
 * the Rust process-monitoring commands (`get_apps_snapshot`) that own this
 * data — keep both sides of that boundary compatible with this shape.
 */
export interface DockApp {
  id: string;
  name: string;
  /** macOS bundle ID or Windows canonical `.exe` path — sent to `launch_or_activate_app`. */
  bundleId: string;
  /**
   * Native icon rendered by Rust as a `data:image/png;base64,...` URL, or
   * `null` if it couldn't be resolved (app not installed). Always pair with
   * a fallback — see DockIcon.
   */
  iconUrl: string | null;
  /** Whether the app is currently running, per `NSWorkspace` — not mocked. */
  isActive: boolean;
  /** Resolved LED hex — accounts for global mode, auto sample, and override. */
  indicatorColor: string;
  /** Auto-sampled accent from the native icon (or hash fallback). */
  indicatorColorAuto: string;
  /** Manual `#rrggbb` override, or `null` to use auto/global mode. */
  indicatorColorOverride: string | null;
}

/** One dock row — app (with runtime fields) or a visual separator. */
export type DockItem =
  | ({ type: "app" } & DockApp)
  | { type: "separator"; id: string };

export function isDockAppItem(
  item: DockItem,
): item is { type: "app" } & DockApp {
  return item.type === "app";
}

export function countDockApps(items: DockItem[]): number {
  return items.filter(isDockAppItem).length;
}

export function countDockSeparators(items: DockItem[]): number {
  return items.filter((item) => item.type === "separator").length;
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
 * Which screen edge the dock is anchored to — mirrors `DockPosition` in
 * `src-tauri/src/commands/settings.rs`.
 */
export type DockPosition = "bottom" | "top" | "left" | "right";

/** Z-order of the dock — mirrors `DockWindowLayer` in settings.rs. */
export type DockWindowLayer = "above_windows" | "below_windows";

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
  /** Id into `BORDER_STYLE_PRESETS` (constants.ts) — picks which keyframe
   * animation drives the RGB frame while `animationsEnabled` is on:
   * smooth spectrum cycle, neon breathing pulse, glitch flicker, or a
   * rotating radar-style scan ring. Falls back to the first preset for an
   * unrecognized id, same convention as `backgroundPreset`. */
  borderStyle: string;
  /** Perimeter frame width in logical px (1–8). Drives the unified gradient
   * ring overlay (`--dock-border-width`) and the pill's static border when
   * animations are off. */
  borderWidthPx: number;
  /** Master toggle for the panel-body decorative overlay (scanlines / HUD
   * grid / hologram flicker) — independent of the border cycle and the
   * background gradient flow, each of which has its own on/off. */
  panelEffectEnabled: boolean;
  /** Id into `PANEL_EFFECT_PRESETS` (constants.ts) — which overlay
   * animation plays across the pill body, tinted from the active
   * background preset's colors. */
  panelEffect: string;
  /** Master toggle for the animated RGB/gradient background layer under
   * the icons (painted on top of the native vibrancy blur). Independent
   * of `animationsEnabled`, which only ever covered the border cycle + LED
   * pulse — this is a separate decorative layer with its own on/off. */
  backgroundAnimationEnabled: boolean;
  /** Id into `BACKGROUND_PRESETS` (constants.ts) — picks the gradient's 6
   * color stops. Falls back to the first preset if unrecognized (e.g. an
   * older config file). */
  backgroundPreset: string;
  /** 0..1 — how vivid/bright the preset's colors render (mixed toward
   * black at 0, full color at 1). */
  backgroundIntensity: number;
  /** 0..1 — opacity of the whole gradient layer over the glass. */
  backgroundVisibility: number;
  /** 0..1 — flow speed; mapped to an animation duration via
   * `backgroundSpeedToDurationS`. */
  backgroundSpeed: number;
  /** Id into `ICON_SIZE_PRESETS` (constants.ts) — quick snap buttons in
   * settings; geometry itself is driven by `iconSizePx`. */
  iconSizePreset: string;
  /** Icon edge length in logical px (44–72) — single input every dock layout
   * number derives from via `getSizeMetrics`. */
  iconSizePx: number;
  /** 0..1 — how much hover-magnify spreads to neighboring icons. At 0 only
   * the icon under the cursor grows; at 1 the current full neighbor curve. */
  magnifyNeighborStrength: number;
  /** How running-app LED colors are chosen: auto from icon, one fixed color,
   * or manual overrides only. */
  ledColorMode: "auto" | "fixed" | "override_only";
  /** Hex LED color when `ledColorMode` is `fixed`. */
  ledFixedColor: string;
  /** Which screen edge the dock is anchored to — see `DockPosition`. */
  dockPosition: DockPosition;
  /** Whether the dock stays above app windows or sits below them. */
  dockWindowLayer: DockWindowLayer;
  /** Windows-only: chrome HUD on the dock + denser `[win-diag]` logs. */
  windowsDebugOverlay: boolean;
  /**
   * Windows-only: GDI `SetWindowRgn` pill clip plus the 2px paint inset /
   * soft edge masks in `index.css`. Default `true` — hides pale crescents
   * after focus when per-pixel alpha flickers. Set `false` for CSS-only
   * soft corners (crisper RGB AA when alpha is stable on that machine).
   */
  windowsHardClip: boolean;
}
