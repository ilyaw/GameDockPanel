import type { DockSettings } from "./types";

/**
 * Dock layout numbers — single source of truth for JS magnify math and for
 * keeping Rust window sizing / hit-testing (platform/macos.rs) in sync.
 * Everything that scales with icon size flows through `getSizeMetrics`
 * (fed by `DockSettings.iconSizePx`) instead of being a fixed module-level
 * constant.
 */

/** Continuous icon-size slider bounds — presets snap to the endpoints and middle. */
export const ICON_SIZE_MIN_PX = 44;
export const ICON_SIZE_MAX_PX = 72;
export const DEFAULT_ICON_SIZE_PX = 56;

/** macOS-like squircle proportion — scales with `iconSizePx` so compact icons
 * don't keep an oversized fixed corner radius that looks soft/blurry. */
export const ICON_CORNER_RADIUS_RATIO = 0.22;

/**
 * Selectable icon sizes — `id` round-trips through
 * `DockSettings.iconSizePreset`; `iconSizePx` is the single number
 * `getSizeMetrics` scales every other layout value from. `medium` (56) is
 * the original fixed size this dock shipped with, kept as the default so
 * existing installs don't visually change until the user picks another one.
 */
export interface IconSizePreset {
  id: string;
  label: string;
  /** Matches `BorderStylePreset`/`PanelEffectPreset`'s shape so the settings
   * UI can reuse the same `StylePresetButton` for all three preset rows. */
  description: string;
  iconSizePx: number;
}

export const ICON_SIZE_PRESETS: IconSizePreset[] = [
  {
    id: "small",
    label: "Компакт",
    description: "Маленькие иконки — более компактная и плотная панель.",
    iconSizePx: 44,
  },
  {
    id: "medium",
    label: "Стандарт",
    description: "Исходный размер панели.",
    iconSizePx: 56,
  },
  {
    id: "large",
    label: "Крупный",
    description: "Крупные иконки — просторнее и проще попасть курсором.",
    iconSizePx: 72,
  },
];

/** Falls back to `medium` for an unrecognized id — same "future config,
 * older build" guard as `getBackgroundPreset`. */
export function getIconSizePreset(id: string): IconSizePreset {
  return ICON_SIZE_PRESETS.find((preset) => preset.id === id) ?? ICON_SIZE_PRESETS[1];
}

/** Maps a legacy preset id to its canonical px — used when loading older
 * `dock-settings.json` files that predate `iconSizePx`. */
export function iconSizePxFromPreset(id: string): number {
  return getIconSizePreset(id).iconSizePx;
}

/** Clamps a raw slider/commit value into the allowed icon-size range. */
export function clampIconSizePx(px: number): number {
  return Math.round(Math.min(ICON_SIZE_MAX_PX, Math.max(ICON_SIZE_MIN_PX, px)));
}

/** Reference icon size the scale-dependent constants below were originally
 * tuned against (the dock's pre-preset fixed size) — `getSizeMetrics`
 * scales them by `iconSizePx / BASE_ICON_SIZE_PX`. */
const BASE_ICON_SIZE_PX = 56;
/** Mirrors the original `gap-2` between icons in the dock pill. */
const BASE_DOCK_GAP_PX = 8;
/** Mirrors the original `px-5` on the dock pill. */
const BASE_DOCK_PADDING_X_PX = 20;
/** Mirrors the original `py-3` on the dock pill (rest vertical padding). */
const BASE_DOCK_PADDING_Y_PX = 12;
/** Mirrors the original `gap-2` between icon and LED. */
const BASE_ICON_LED_GAP_PX = 8;

/**
 * Upper bound for hover-magnify scale (`origin-bottom` on the icon) — a
 * ratio, not a pixel value, so it applies unchanged at every preset. The
 * *pixel* amplitude it produces (`magnifyHeightOverflowPx` in
 * `SizeMetrics`) already scales on its own because it multiplies against
 * `iconSizePx`, so this doesn't need its own per-preset value.
 */
export const MAGNIFY_MAX_SCALE = 1.4;

/**
 * Fixed across every icon-size preset — like the real macOS Dock, whose
 * own size slider doesn't change the pill's corner shape, its margin to
 * the screen edge, glow bleed, divider thickness, or the LED indicator
 * (already independent of `ICON_SIZE_PX` before presets existed), only the
 * icon content. Tooltip/context-menu row heights are fixed for the same
 * reason as LED size: their font size doesn't scale with icon size.
 */
export const DOCK_BOTTOM_INSET_PX = 8;
export const LED_HEIGHT_PX = 3;
/** Mirrors the vertical divider between the app icons and the settings
 * gear in DockPanel.tsx (`mx-1 w-px`) — 4px margin each side + 1px line. */
export const DOCK_DIVIDER_WIDTH_PX = 9;
/** Must match Tailwind's `rounded-[28px]` on the dock pill (DockPanel.tsx). */
export const PILL_CORNER_RADIUS_PX = 28;
/** Horizontal glow bleed (~14px box-shadow each side). */
export const WINDOW_GLOW_BLEED_PX = 32;
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
 * + divider + indicator color + optional Reset + optional Quit divider/row.
 * Must be accounted for in `pillTopReservePx` — the menu renders
 * `bottom-full` off the same anchor, and the window has no scroll, so
 * anything past its height is simply not drawn.
 */
export const CONTEXT_MENU_MAX_ROWS = 5;
export const CONTEXT_MENU_MAX_DIVIDERS = 2;
export const CONTEXT_MENU_HEIGHT_PX =
  CONTEXT_MENU_ROW_HEIGHT_PX * CONTEXT_MENU_MAX_ROWS +
  CONTEXT_MENU_DIVIDER_HEIGHT_PX * CONTEXT_MENU_MAX_DIVIDERS;

/**
 * Soft ceiling on total dock entries (seeded + manually added) — mirrors
 * `MAX_APPS` in src-tauri/src/commands/apps.rs. Past this limit, drag-drop
 * from Finder is rejected on the Rust side.
 */
export const MAX_APPS = 15;

/**
 * Every dock layout number that DOES depend on icon size, computed from one
 * input by `getSizeMetrics`. Kept as one object (instead of a dozen loose
 * module-level consts, as before presets existed) so Rust's `size_metrics`
 * in `platform/macos.rs` can mirror it field-for-field.
 */
export interface SizeMetrics {
  iconSizePx: number;
  /** Proportional squircle radius for app icons and the settings gear slot. */
  iconCornerRadiusPx: number;
  dockGapPx: number;
  dockPaddingXPx: number;
  dockPaddingYPx: number;
  iconLedGapPx: number;
  /** How far a peak-magnified icon grows above its rest top (`origin-bottom`). */
  magnifyHeightOverflowPx: number;
  /** Cursor-to-icon-center distance (px, viewport coords) beyond which
   * magnify falls back to rest scale (1). Spans roughly two neighboring
   * icons on each side of the one closest to the cursor. */
  magnifyInfluenceRadiusPx: number;
  /** Pill outer height at rest: py + icon + gap + LED; magnify overflows above. */
  pillHeightPx: number;
  /** Native hit-test band above the pill (magnify overflow, not CSS pill height). */
  pillHeightHoverPx: number;
  /**
   * Transparent band above the pill inside the window — big enough for
   * whichever thing currently pokes highest above it: a magnified icon,
   * the hover tooltip, or the (taller) context menu. These never show at
   * the same time (menu replaces tooltip; magnify is suppressed while the
   * menu is open), so `max`, not a sum, is the right combinator.
   */
  pillTopReservePx: number;
  /** Tauri window logical height — keep in sync with `window_height_dip`
   * in src-tauri/src/platform/macos.rs and tauri.conf.json. */
  windowHeightDip: number;
}

export function getSizeMetrics(iconSizePx: number): SizeMetrics {
  const scale = iconSizePx / BASE_ICON_SIZE_PX;
  const dockGapPx = BASE_DOCK_GAP_PX * scale;
  const dockPaddingXPx = BASE_DOCK_PADDING_X_PX * scale;
  const dockPaddingYPx = BASE_DOCK_PADDING_Y_PX * scale;
  const iconLedGapPx = BASE_ICON_LED_GAP_PX * scale;

  const magnifyHeightOverflowPx = iconSizePx * (MAGNIFY_MAX_SCALE - 1);
  const magnifyInfluenceRadiusPx = (iconSizePx + dockGapPx) * 2;

  const pillHeightPx = dockPaddingYPx * 2 + iconSizePx + iconLedGapPx + LED_HEIGHT_PX;
  const pillHeightHoverPx = pillHeightPx + magnifyHeightOverflowPx;

  const pillTopReservePx =
    Math.max(
      magnifyHeightOverflowPx,
      TOOLTIP_GAP_PX + TOOLTIP_HEIGHT_PX,
      TOOLTIP_GAP_PX + CONTEXT_MENU_HEIGHT_PX,
    ) - dockPaddingYPx;

  const windowHeightDip = DOCK_BOTTOM_INSET_PX + pillHeightPx + pillTopReservePx;

  return {
    iconSizePx,
    iconCornerRadiusPx: iconSizePx * ICON_CORNER_RADIUS_RATIO,
    dockGapPx,
    dockPaddingXPx,
    dockPaddingYPx,
    iconLedGapPx,
    magnifyHeightOverflowPx,
    magnifyInfluenceRadiusPx,
    pillHeightPx,
    pillHeightHoverPx,
    pillTopReservePx,
    windowHeightDip,
  };
}

/**
 * Pill outer width at rest for `appCount` icons at the given icon size
 * (padding + icons + gaps) — the CSS pill itself is content-driven (no
 * explicit width is ever set in JSX, flex sizes it from its children), so
 * nothing in the frontend actually calls this at runtime. It exists purely
 * as the documented formula that `pill_width_dip`/`window_width_dip` in
 * src-tauri/src/platform/macos.rs mirror for native window/vibrancy/
 * hit-test sizing — kept here, parametrized by count and icon size instead
 * of fixed constants, so the two sides stay provably in sync. The actual
 * on-screen pill's vibrancy blur mask is independently corrected from the
 * measured DOM rect at runtime (see `sync_vibrancy_pill` /
 * `commands/window.rs`) — this formula only has to get the *native window
 * frame* wide enough to avoid clipping it.
 */
export function pillWidthPx(appCount: number, iconSizePx: number): number {
  const metrics = getSizeMetrics(iconSizePx);
  const appsWidth = appCount * iconSizePx + (appCount - 1) * metrics.dockGapPx;
  // Trailing divider + settings gear in DockPanel — gap, divider, gap, icon.
  const settingsSlot =
    metrics.dockGapPx + DOCK_DIVIDER_WIDTH_PX + metrics.dockGapPx + iconSizePx;
  return metrics.dockPaddingXPx * 2 + appsWidth + settingsSlot;
}

/** See `pillWidthPx` — same "documented formula, not called at runtime" note. */
export function windowWidthDip(appCount: number, iconSizePx: number): number {
  return (
    pillWidthPx(appCount, iconSizePx) +
    Math.ceil(iconSizePx * (MAGNIFY_MAX_SCALE - 1)) +
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
  {
    id: "vapor",
    label: "Vaporwave",
    colors: ["#ff71ce", "#b967ff", "#01cdfe", "#05ffa1", "#fffb96", "#ff71ce"],
  },
  {
    id: "matrix",
    label: "Matrix",
    colors: ["#003b00", "#008f11", "#00ff41", "#39ff14", "#00ff41", "#008f11"],
  },
  {
    id: "plasma",
    label: "Plasma",
    colors: ["#7b2ff7", "#f107a3", "#ff6b6b", "#feca57", "#f107a3", "#7b2ff7"],
  },
  {
    id: "bloodmoon",
    label: "Blood Moon",
    colors: ["#1a0000", "#8b0000", "#ff0000", "#ff4500", "#8b0000", "#1a0000"],
  },
  {
    id: "neon-noir",
    label: "Neon Noir",
    colors: ["#ff00ff", "#8000ff", "#0080ff", "#ff00aa", "#4b0082", "#ff00ff"],
  },
  {
    id: "solar",
    label: "Solar Flare",
    colors: ["#ffd700", "#ffae00", "#ff7b00", "#fff2cc", "#ffae00", "#ffd700"],
  },
];

/** Falls back to the first preset for an unrecognized id — e.g. a config
 * file written by a future version with a preset this build doesn't know
 * about yet, rather than rendering nothing. */
export function getBackgroundPreset(id: string): BackgroundPreset {
  return BACKGROUND_PRESETS.find((preset) => preset.id === id) ?? BACKGROUND_PRESETS[0];
}

/**
 * A cyberpunk-styled animation driving the pill's RGB frame while
 * `DockSettings.animationsEnabled` is on — picked by id in
 * `DockSettings.borderStyle`. `animationClass` names a Tailwind
 * `--animate-*` utility registered in `src/index.css`; `"scan"` is the one
 * exception — it renders no border-color keyframe of its own and instead
 * gets a dedicated rotating conic-gradient ring overlay in `DockPanel.tsx`
 * (see `dock-border-scan-ring` in index.css), so its `animationClass` is
 * left empty.
 */
export interface BorderStylePreset {
  id: string;
  label: string;
  description: string;
  animationClass: string;
}

export const BORDER_STYLE_PRESETS: BorderStylePreset[] = [
  {
    id: "spectrum",
    label: "Спектр",
    description: "Плавный перелив всех 6 цветов по кругу.",
    animationClass: "animate-rgb-glow",
  },
  {
    id: "pulse",
    label: "Пульс",
    description: "Неоновое дыхание — рамка разгорается и затухает в такт.",
    animationClass: "animate-border-pulse",
  },
  {
    id: "glitch",
    label: "Глитч",
    description: "Рваные скачки цвета и короткие обрывы сигнала.",
    animationClass: "animate-border-glitch",
  },
  {
    id: "scan",
    label: "Скан",
    description: "Луч радара, вращающийся по периметру рамки.",
    animationClass: "",
  },
];

/** Falls back to the first preset for an unrecognized id — same
 * "future config, older build" guard as `getBackgroundPreset`. */
export function getBorderStylePreset(id: string): BorderStylePreset {
  return BORDER_STYLE_PRESETS.find((preset) => preset.id === id) ?? BORDER_STYLE_PRESETS[0];
}

/**
 * A decorative overlay animation for the pill body itself (not just its
 * frame) — picked by id in `DockSettings.panelEffect`, gated by
 * `DockSettings.panelEffectEnabled`. Tinted at render time from the active
 * `BackgroundPreset`'s colors (`--dock-bg-*` custom properties already set
 * on the pill for the background flow layer), so it never needs its own
 * separate color config.
 */
export interface PanelEffectPreset {
  id: string;
  label: string;
  description: string;
}

export const PANEL_EFFECT_PRESETS: PanelEffectPreset[] = [
  { id: "none", label: "Нет", description: "Без дополнительного слоя поверх панели." },
  {
    id: "scanline",
    label: "Скан-линии",
    description: "Тонкие горизонтальные линии в стиле ЭЛТ-монитора — без блика по иконкам.",
  },
  {
    id: "grid",
    label: "HUD-сетка",
    description: "Тонкая киберпанк-сетка, медленно смещающаяся по панели.",
  },
  {
    id: "flicker",
    label: "Голограмма",
    description: "Мерцание в стиле нестабильной голографической проекции.",
  },
];

/** Same fallback convention as `getBackgroundPreset`/`getBorderStylePreset`. */
export function getPanelEffectPreset(id: string): PanelEffectPreset {
  return PANEL_EFFECT_PRESETS.find((preset) => preset.id === id) ?? PANEL_EFFECT_PRESETS[0];
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
  borderStyle: "spectrum",
  panelEffectEnabled: true,
  panelEffect: "grid",
  backgroundAnimationEnabled: true,
  backgroundPreset: "chroma",
  backgroundIntensity: 0.7,
  backgroundVisibility: 0.45,
  backgroundSpeed: 0.4,
  iconSizePreset: "medium",
  iconSizePx: DEFAULT_ICON_SIZE_PX,
  ledColorMode: "auto",
  ledFixedColor: "#ff9d3b",
};

export type LedColorMode = DockSettings["ledColorMode"];

export const LED_COLOR_MODE_OPTIONS: {
  id: LedColorMode;
  label: string;
  description: string;
}[] = [
  {
    id: "auto",
    label: "Из иконки",
    description: "Автоматически подбирает цвет из иконки приложения",
  },
  {
    id: "fixed",
    label: "Один цвет",
    description: "Одинаковый цвет индикатора для всех приложений",
  },
  {
    id: "override_only",
    label: "Только ручные",
    description: "Только приложения с заданным вручную цветом; остальные — нейтральные",
  },
];
