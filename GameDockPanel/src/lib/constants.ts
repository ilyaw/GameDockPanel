/**
 * Dock layout numbers — single source of truth for JS magnify math and for
 * keeping Rust window sizing / hit-testing (platform/macos.rs) in sync.
 */

/** Mirrors `h-14 w-14` on the icon image/fallback badge. */
export const ICON_SIZE_PX = 56;

/** Mirrors `gap-5` between icons in the dock pill. */
export const DOCK_GAP_PX = 20;

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

/** Gap between tooltip bottom edge and icon top (`margin-bottom` on tooltip). */
export const TOOLTIP_GAP_PX = 16;

/** Approximate rendered height of the hover name tooltip (`text-xs` + `py-1`). */
export const TOOLTIP_HEIGHT_PX = 28;

/**
 * Cursor-to-icon-center distance (px, viewport coords) beyond which magnify
 * falls back to rest scale (1). Spans roughly two neighboring icons on each
 * side of the one closest to the cursor.
 */
export const MAGNIFY_INFLUENCE_RADIUS_PX = (ICON_SIZE_PX + DOCK_GAP_PX) * 2;

/** Fixed dock roster size (Discord, Steam, Spotify, Minecraft, OBS, Epic Games). */
export const APP_COUNT = 6;

/** Pill outer height at rest: py + icon + gap + LED — fixed; magnify overflows above. */
export const PILL_HEIGHT_PX =
  DOCK_PADDING_Y_PX * 2 + ICON_SIZE_PX + ICON_LED_GAP_PX + LED_HEIGHT_PX;

/** Native hit-test band above the fixed pill (magnify overflow, not CSS pill height). */
export const PILL_HEIGHT_HOVER_PX =
  PILL_HEIGHT_PX + MAGNIFY_HEIGHT_OVERFLOW_PX;

/** Pill outer width at rest (padding + icons + gaps). */
export const PILL_WIDTH_PX =
  DOCK_PADDING_X_PX * 2 +
  APP_COUNT * ICON_SIZE_PX +
  (APP_COUNT - 1) * DOCK_GAP_PX;

/**
 * Transparent band above the fixed pill inside the window — magnify overflow
 * plus any tooltip that sticks out above enlarged icons.
 */
export const PILL_TOP_RESERVE_PX =
  MAGNIFY_HEIGHT_OVERFLOW_PX +
  TOOLTIP_GAP_PX +
  TOOLTIP_HEIGHT_PX -
  DOCK_PADDING_Y_PX;

/**
 * Tauri window logical size — keep in sync with `WINDOW_*_DIP` in
 * src-tauri/src/platform/macos.rs and tauri.conf.json.
 */
export const WINDOW_HEIGHT_DIP =
  DOCK_BOTTOM_INSET_PX + PILL_HEIGHT_PX + PILL_TOP_RESERVE_PX;

/** Horizontal glow bleed (~14px box-shadow each side). */
export const WINDOW_GLOW_BLEED_PX = 32;

export const WINDOW_WIDTH_DIP =
  PILL_WIDTH_PX +
  Math.ceil(ICON_SIZE_PX * (MAGNIFY_MAX_SCALE - 1)) +
  WINDOW_GLOW_BLEED_PX;
