import type { DockSettings } from "./types";

/**
 * Dock layout numbers — single source of truth for JS magnify math and for
 * keeping Rust window sizing / hit-testing (platform/macos.rs) in sync.
 */

/** Mirrors `h-14 w-14` on the icon image/fallback badge. */
export const ICON_SIZE_PX = 56;

/** Mirrors `gap-2` between icons in the dock pill. */
export const DOCK_GAP_PX = 8;

/** Mirrors `px-5` on the dock pill. */
export const DOCK_PADDING_X_PX = 20;

/** Mirrors `py-3` on the dock pill (rest vertical padding). */
export const DOCK_PADDING_Y_PX = 12;

/** Upper bound for hover-magnify scale (`origin-bottom` on the icon). */
export const MAGNIFY_MAX_SCALE = 1.4;

/** How far a peak-magnified icon grows above its rest top (`origin-bottom`). */
export const MAGNIFY_HEIGHT_OVERFLOW_PX = Math.ceil(
  ICON_SIZE_PX * (MAGNIFY_MAX_SCALE - 1),
);

/** Mirrors `pb-2` — gap between pill and the window bottom edge. */
export const DOCK_BOTTOM_INSET_PX = 8;

/** Mirrors `gap-2` between icon and LED. */
export const ICON_LED_GAP_PX = 8;

/** LED bar height (`h-[3px]`). */
export const LED_HEIGHT_PX = 3;

/** Gap between tooltip/menu bottom edge and icon top (`margin-bottom` on both). */
export const TOOLTIP_GAP_PX = 16;

/** Approximate rendered height of the hover name tooltip (`text-xs` + `py-1`). */
export const TOOLTIP_HEIGHT_PX = 28;

/**
 * Approximate rendered height of one context-menu row (`DockIcon.tsx`) —
 * `text-xs` row with an icon, `py-1.5` (taller than the tooltip's `py-1`).
 * Rounded up a little over the raw text+padding math as headroom, since
 * under-reserving here clips the menu against the window's fixed height
 * with no visible error (see `CONTEXT_MENU_HEIGHT_PX`).
 */
export const CONTEXT_MENU_ROW_HEIGHT_PX = 30;

/** Thin `h-px` divider above Quit (only rendered while the app is running). */
export const CONTEXT_MENU_DIVIDER_HEIGHT_PX = 1;

/**
 * Tallest the icon context menu ever gets: Show in Finder + Remove from Dock
 * + divider + Quit (Quit and the divider only render while the app is
 * running). Must be
 * accounted for in `PILL_TOP_RESERVE_PX` same as the tooltip — the menu
 * renders `bottom-full` off the same anchor, and the window has no scroll,
 * so anything past its fixed height is simply not drawn.
 */
export const CONTEXT_MENU_HEIGHT_PX =
  CONTEXT_MENU_ROW_HEIGHT_PX * 3 + CONTEXT_MENU_DIVIDER_HEIGHT_PX;

/**
 * Cursor-to-icon-center distance (px, viewport coords) beyond which magnify
 * falls back to rest scale (1). Spans roughly two neighboring icons on each
 * side of the one closest to the cursor.
 */
export const MAGNIFY_INFLUENCE_RADIUS_PX = (ICON_SIZE_PX + DOCK_GAP_PX) * 2;

/**
 * Soft ceiling on total dock entries (seeded + manually added) — mirrors
 * `MAX_APPS` in src-tauri/src/commands/apps.rs. Past this limit, drag-drop
 * from Finder is rejected on the Rust side.
 */
export const MAX_APPS = 15;

/** Pill outer height at rest: py + icon + gap + LED — fixed; magnify overflows above. */
export const PILL_HEIGHT_PX =
  DOCK_PADDING_Y_PX * 2 + ICON_SIZE_PX + ICON_LED_GAP_PX + LED_HEIGHT_PX;

/** Native hit-test band above the fixed pill (magnify overflow, not CSS pill height). */
export const PILL_HEIGHT_HOVER_PX =
  PILL_HEIGHT_PX + MAGNIFY_HEIGHT_OVERFLOW_PX;

/**
 * Transparent band above the fixed pill inside the window — big enough for
 * whichever thing currently pokes highest above the pill: a magnified icon,
 * the hover tooltip, or the (taller) context menu. These never show at the
 * same time (menu replaces tooltip; magnify is suppressed while the menu is
 * open — see `isHovered || menuOpen` / dragging guards in `DockIcon.tsx`),
 * so `max`, not a sum, is the right combinator — summing would reserve far
 * more transparent dead space above the dock than anything ever needs.
 */
export const PILL_TOP_RESERVE_PX =
  Math.max(
    MAGNIFY_HEIGHT_OVERFLOW_PX,
    TOOLTIP_GAP_PX + TOOLTIP_HEIGHT_PX,
    TOOLTIP_GAP_PX + CONTEXT_MENU_HEIGHT_PX,
  ) - DOCK_PADDING_Y_PX;

/**
 * Tauri window logical size — keep in sync with `WINDOW_HEIGHT_DIP` in
 * src-tauri/src/platform/macos.rs and tauri.conf.json. Height never depends
 * on app count — only width does (see `pillWidthPx`/`windowWidthDip` below).
 */
export const WINDOW_HEIGHT_DIP =
  DOCK_BOTTOM_INSET_PX + PILL_HEIGHT_PX + PILL_TOP_RESERVE_PX;

/** Horizontal glow bleed (~14px box-shadow each side). */
export const WINDOW_GLOW_BLEED_PX = 32;

/**
 * Pill outer width at rest for `appCount` icons (padding + icons + gaps) —
 * the CSS pill itself is content-driven (no explicit width is ever set in
 * JSX, flex sizes it from its children), so nothing in the frontend
 * actually calls this at runtime. It exists purely as the documented
 * formula that `pill_width_dip`/`window_width_dip` in
 * src-tauri/src/platform/macos.rs mirror for native window/vibrancy/
 * hit-test sizing — kept here, parametrized by count instead of a fixed
 * `APP_COUNT`, so the two sides stay provably in sync. The actual
 * on-screen pill's vibrancy blur mask is independently corrected from the
 * measured DOM rect at runtime (see `sync_vibrancy_pill` /
 * `commands/window.rs`) — this formula only has to get the *native window
 * frame* wide enough to avoid clipping it.
 */
export function pillWidthPx(appCount: number): number {
  const appsWidth =
    appCount * ICON_SIZE_PX + (appCount - 1) * DOCK_GAP_PX;
  // Trailing settings gear in DockPanel — one gap + one icon slot.
  const settingsSlot = DOCK_GAP_PX + ICON_SIZE_PX;
  return DOCK_PADDING_X_PX * 2 + appsWidth + settingsSlot;
}

/** See `pillWidthPx` — same "documented formula, not called at runtime" note. */
export function windowWidthDip(appCount: number): number {
  return (
    pillWidthPx(appCount) +
    Math.ceil(ICON_SIZE_PX * (MAGNIFY_MAX_SCALE - 1)) +
    WINDOW_GLOW_BLEED_PX
  );
}

/**
 * A ready-made gamer-style RGB/gradient combo for the dock's animated
 * background layer — picked by id in `DockSettings.backgroundPreset`.
 * Fixed at 6 stops, mirroring the border cycle's own `rgbGlowColors`
 * convention (`RGB_GLOW_COLOR_COUNT` in settings.rs) purely for
 * consistency; unlike that field, presets aren't user-editable, so no
 * length validation is needed on the Rust side — only an id round-trips
 * through persistence, this color data lives on the frontend alone.
 */
export interface BackgroundPreset {
  id: string;
  label: string;
  colors: [string, string, string, string, string, string];
}

export const BACKGROUND_PRESETS: BackgroundPreset[] = [
  {
    id: "chroma",
    label: "Chroma",
    colors: ["#ff3b6b", "#ff9d3b", "#e9ff3b", "#3bffb0", "#3bb0ff", "#b03bff"],
  },
  {
    id: "cyberpunk",
    label: "Cyberpunk",
    colors: ["#ff2ec4", "#8b2fff", "#2f6bff", "#00e5ff", "#39ffd8", "#ff2ec4"],
  },
  {
    id: "toxic",
    label: "Toxic",
    colors: ["#caff3f", "#39ff14", "#0aff99", "#00e6b8", "#7bff3f", "#caff3f"],
  },
  {
    id: "inferno",
    label: "Inferno",
    colors: ["#ff003c", "#ff6a00", "#ffb700", "#ff2e00", "#ff8a00", "#ff003c"],
  },
  {
    id: "aurora",
    label: "Aurora",
    colors: ["#00ffd5", "#4dd0ff", "#6a5cff", "#c66aff", "#ff6ec7", "#00ffd5"],
  },
  {
    id: "frost",
    label: "Frost",
    colors: ["#0072ff", "#00c6ff", "#7de2fc", "#c2f5ff", "#66d9ff", "#0072ff"],
  },
];

/** Falls back to the first preset for an unrecognized id — e.g. a config
 * file written by a future version with a preset this build doesn't know
 * about yet, rather than rendering nothing. */
export function getBackgroundPreset(id: string): BackgroundPreset {
  return BACKGROUND_PRESETS.find((preset) => preset.id === id) ?? BACKGROUND_PRESETS[0];
}

/** Animation-duration bounds for the background flow, in seconds — the
 * `backgroundSpeed` slider (0..1) maps onto this range inversely (1 =
 * fastest = shortest duration) via `backgroundSpeedToDurationS`. */
export const BACKGROUND_FLOW_MIN_DURATION_S = 4;
export const BACKGROUND_FLOW_MAX_DURATION_S = 24;

export function backgroundSpeedToDurationS(speed: number): number {
  return (
    BACKGROUND_FLOW_MAX_DURATION_S -
    speed * (BACKGROUND_FLOW_MAX_DURATION_S - BACKGROUND_FLOW_MIN_DURATION_S)
  );
}

/**
 * Mirrors `DockSettings::default()` in `src-tauri/src/commands/settings.rs`
 * — used as `useDockSettings`'s initial state so the very first render
 * already matches what the backend will report a moment later for a user
 * who has never touched settings, instead of flashing some other filler
 * value before the async `get_dock_settings` round-trip resolves.
 */
export const DEFAULT_DOCK_SETTINGS: DockSettings = {
  animationsEnabled: true,
  rgbGlowColors: ["#ff3b6b", "#ff9d3b", "#e9ff3b", "#3bffb0", "#3bb0ff", "#b03bff"],
  staticGlowColor: "#ff3b6b",
  backgroundAnimationEnabled: true,
  backgroundPreset: "chroma",
  backgroundIntensity: 0.7,
  backgroundVisibility: 0.45,
  backgroundSpeed: 0.4,
};
