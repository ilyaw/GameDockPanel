import type { DockItem, DockSettings } from "./types";

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
 * Upper bound for hover-magnify scale — a ratio, not a pixel value, so it
 * applies unchanged at every preset. Transform-origin is set per
 * `dockPosition` (see `useDockOrientation`) so the icon grows toward the
 * desktop center. The *pixel* amplitude (`magnifyHeightOverflowPx` in
 * `SizeMetrics`) scales with `iconSizePx`.
 */
export const MAGNIFY_MAX_SCALE = 1.4;

/** Falloff exponent at `magnifyNeighborStrength === 1` (linear, current curve). */
export const MAGNIFY_FALLOFF_EXP_MIN = 1;
/** Falloff exponent at `magnifyNeighborStrength === 0` (neighbors stay at rest). */
export const MAGNIFY_FALLOFF_EXP_MAX = 10;
export const DEFAULT_MAGNIFY_NEIGHBOR_STRENGTH = 1;

/** Maps slider 0..1 to the `(1 - t)^exp` exponent used by magnify. */
export function magnifyFalloffExponent(neighborStrength: number): number {
  const strength = Math.min(1, Math.max(0, neighborStrength));
  return (
    MAGNIFY_FALLOFF_EXP_MAX +
    (MAGNIFY_FALLOFF_EXP_MIN - MAGNIFY_FALLOFF_EXP_MAX) * strength
  );
}

/** Hover-magnify scale from cursor-to-icon-center distance on the main axis. */
export function computeMagnifyScale(
  absDistancePx: number,
  radiusPx: number,
  neighborStrength: number,
): number {
  if (radiusPx <= 0) return 1;
  const t = absDistancePx / radiusPx;
  if (t >= 1) return 1;
  const exp = magnifyFalloffExponent(neighborStrength);
  return 1 + (MAGNIFY_MAX_SCALE - 1) * (1 - t) ** exp;
}

/**
 * Peak launch-bounce `translateY` as a fraction of `magnifyHeightOverflowPx`
 * — deliberately tied to the *same* number `pillFarReservePx` already
 * reserves headroom for, instead of an independent constant, so bounce
 * amplitude + peak magnify overflow happening at once can never exceed
 * that existing reserve (dominated in practice by
 * `CONTEXT_MENU_HEIGHT_PX`). Keeps this a pure frontend visual number that
 * never has to grow the window or get mirrored into `platform/macos.rs` —
 * same rationale already documented for `magnifyInfluenceRadiusPx`.
 */
export const LAUNCH_BOUNCE_AMPLITUDE_RATIO = 0.55;

/**
 * Fixed across every icon-size preset — like the real macOS Dock, whose
 * own size slider doesn't change the pill's corner shape, its margin to
 * the screen edge, glow bleed, divider thickness, or the LED indicator
 * (already independent of `ICON_SIZE_PX` before presets existed), only the
 * icon content. Tooltip/context-menu row heights are fixed for the same
 * reason as LED size: their font size doesn't scale with icon size.
 */
/** Gap between the dock pill's near edge and the screen edge it's anchored
 * to — bottom edge for `dockPosition: "bottom"`, top for `"top"`, etc.
 * Named generically since Phase 1 (PROMPT_15_POSITION_PHASE1.md)
 * generalized anchoring beyond bottom-only. Mirrors `DOCK_EDGE_INSET_DIP`
 * in src-tauri/src/platform/macos.rs. */
export const DOCK_EDGE_INSET_PX = 8;
export const LED_HEIGHT_PX = 3;
/** Edge-running-app dot for `left`/`right` docks — painted into the pill's
 * near-edge padding, not beside the icon in the flex flow. */
export const LED_EDGE_DOT_PX = 6;
/** Min inset of the dot center from the pill's inner near-edge (max toward
 * the screen edge without leaving the dock panel). */
export const LED_EDGE_INSET_FROM_PILL_PX = 3;
/** Horizontal slot for an in-row dock separator — distinct from icon width.
 * Mirrors `DOCK_SEPARATOR_WIDTH_DIP` in src-tauri/src/platform/macos.rs. */
export const DOCK_SEPARATOR_WIDTH_PX = 7;
/** Invisible hit-target width/height along the row axis — wider than the
 * visible 7px slot so magnified neighbors don't steal contextmenu. */
export const DOCK_SEPARATOR_HIT_PX = 18;
/** Vertical row divider height as a fraction of `iconSizePx` — used by
 * in-row dock separators (`DockRowDivider`). */
export const DOCK_ROW_DIVIDER_HEIGHT_RATIO = 0.6;
/** Must match Tailwind's `rounded-[28px]` on the dock pill (DockPanel.tsx). */
export const PILL_CORNER_RADIUS_PX = 28;

/** Unified RGB frame ring thickness — all 9 border styles read
 * `--dock-border-width` on the gradient overlay. */
export const BORDER_WIDTH_MIN_PX = 1;
export const BORDER_WIDTH_MAX_PX = 8;
export const DEFAULT_BORDER_WIDTH_PX = 5;

export function clampBorderWidthPx(px: number): number {
  return Math.round(
    Math.min(BORDER_WIDTH_MAX_PX, Math.max(BORDER_WIDTH_MIN_PX, px)),
  );
}

/** SVG ring mask for gradient border overlays — a stroked rounded rect keeps
 * inner/outer radii parallel at any thickness (innerR ≈ outerR − borderWidth). */
export function roundedRingMaskStyle(
  widthPx: number,
  heightPx: number,
  outerRadiusPx: number,
  borderWidthPx: number,
): {
  maskImage: string;
  WebkitMaskImage: string;
  maskSize: string;
  WebkitMaskSize: string;
} {
  const w = Math.max(1, widthPx);
  const h = Math.max(1, heightPx);
  const bw = Math.max(1, borderWidthPx);
  const half = bw / 2;
  const rx = Math.max(0, Math.min(outerRadiusPx - half, w / 2 - half, h / 2 - half));
  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none">`,
    `<rect x="${half}" y="${half}" width="${w - bw}" height="${h - bw}"`,
    ` rx="${rx}" ry="${rx}" fill="none" stroke="white" stroke-width="${bw}"/>`,
    `</svg>`,
  ].join("");
  const url = `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
  return {
    maskImage: url,
    WebkitMaskImage: url,
    maskSize: "100% 100%",
    WebkitMaskSize: "100% 100%",
  };
}
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
/** Thin `h-px` divider between menu sections. */
export const CONTEXT_MENU_DIVIDER_HEIGHT_PX = 1;
/**
 * Tallest the icon context menu ever gets along the axis it opens on.
 * Main column is short (Finder + Параметры + optional Завершить), but the
 * «Параметры» submenu can be taller (~6 rows + dividers). Keep this reserve
 * at or above that submenu height — runtime `set_menu_overlay` measures the
 * real union bbox (main + open submenu).
 */
export const CONTEXT_MENU_MAX_ROWS = 7;
export const CONTEXT_MENU_MAX_DIVIDERS = 4;
export const CONTEXT_MENU_HEIGHT_PX =
  CONTEXT_MENU_ROW_HEIGHT_PX * CONTEXT_MENU_MAX_ROWS +
  CONTEXT_MENU_DIVIDER_HEIGHT_PX * CONTEXT_MENU_MAX_DIVIDERS;

/**
 * Soft ceiling on total dock entries (seeded + manually added) — mirrors
 * `MAX_APPS` in src-tauri/src/commands/apps.rs. Past this limit, drag-drop
 * from Finder is rejected on the Rust side.
 */
export const MAX_APPS = 15;

/** Soft ceiling on separator count — mirrors `MAX_SEPARATORS` in
 * src-tauri/src/commands/apps.rs. */
export const MAX_SEPARATORS = 5;

/**
 * Every dock layout number that DOES depend on icon size, computed from one
 * input by `getSizeMetrics`. Kept as one object (instead of a dozen loose
 * module-level consts, as before presets existed) so Rust's `size_metrics`
 * in `platform/macos.rs` can mirror it field-for-field.
 *
 * `*ThicknessPx` names the dock's fixed, icon-size-driven axis — CSS
 * height for `dockPosition: "bottom"|"top"`, CSS width for `"left"|"right"`
 * (see `useDockOrientation`). Everything else here (gap, padding, magnify
 * numbers) stays orientation-neutral.
 */
export interface SizeMetrics {
  iconSizePx: number;
  /** Proportional squircle radius for app icons. */
  iconCornerRadiusPx: number;
  dockGapPx: number;
  dockPaddingXPx: number;
  dockPaddingYPx: number;
  iconLedGapPx: number;
  /** How far a peak-magnified icon grows past its rest edge on the magnify
   * axis (magnitude only — direction comes from `magnifyTransformOrigin`). */
  magnifyHeightOverflowPx: number;
  /** Cursor-to-icon-center distance (px, viewport coords) beyond which
   * magnify falls back to rest scale (1). Spans roughly two neighboring
   * icons on each side of the one closest to the cursor. */
  magnifyInfluenceRadiusPx: number;
  /** Peak upward `translateY` for the launch-bounce animation (`DockIcon`) —
   * see `LAUNCH_BOUNCE_AMPLITUDE_RATIO` for why this is derived from
   * `magnifyHeightOverflowPx` rather than its own independent constant. */
  launchBounceAmplitudePx: number;
  /** Pill's thickness-axis size at rest. When the LED sits under the icon
   * (`ledAlongThickness`), thickness includes gap + bar; for edge dots on
   * `left`/`right` the dot is absolutely positioned in padding instead. */
  pillThicknessPx: number;
  /** Native hit-test band past the pill's near edge (magnify overflow, not
   * the CSS thickness itself). */
  pillThicknessHoverPx: number;
  /**
   * Transparent band on the far side of the pill (away from the anchored
   * screen edge) inside the window — big enough for whichever thing
   * currently pokes furthest past it: a magnified icon, the hover tooltip,
   * or the (taller) context menu. These never show at the same time (menu
   * replaces tooltip; magnify is suppressed while the menu is open), so
   * `max`, not a sum, is the right combinator. Named `*Top*` for history
   * (this dock only ever anchored to the bottom when it was introduced) —
   * still literally "above the pill" for `dockPosition: "bottom"|"top"`,
   * but for `"left"|"right"` it maps onto the far side of the *thickness*
   * axis — the direction magnify/tooltip/menu grow after Phase 2.
   */
  pillFarReservePx: number;
  /** Tauri window logical size along the thickness axis — keep in sync
   * with `window_thickness_dip` in src-tauri/src/platform/macos.rs and
   * tauri.conf.json. */
  windowThicknessDip: number;
}

export function getSizeMetrics(
  iconSizePx: number,
  options?: { ledAlongThickness?: boolean },
): SizeMetrics {
  const ledAlongThickness = options?.ledAlongThickness ?? true;
  const scale = iconSizePx / BASE_ICON_SIZE_PX;
  const dockGapPx = BASE_DOCK_GAP_PX * scale;
  const dockPaddingXPx = BASE_DOCK_PADDING_X_PX * scale;
  const dockPaddingYPx = BASE_DOCK_PADDING_Y_PX * scale;
  const iconLedGapPx = BASE_ICON_LED_GAP_PX * scale;

  const magnifyHeightOverflowPx = iconSizePx * (MAGNIFY_MAX_SCALE - 1);
  const magnifyInfluenceRadiusPx = (iconSizePx + dockGapPx) * 2;
  const launchBounceAmplitudePx =
    magnifyHeightOverflowPx * LAUNCH_BOUNCE_AMPLITUDE_RATIO;

  const pillThicknessPx = ledAlongThickness
    ? dockPaddingYPx * 2 + iconSizePx + iconLedGapPx + LED_HEIGHT_PX
    : dockPaddingYPx * 2 + iconSizePx;
  const pillThicknessHoverPx = pillThicknessPx + magnifyHeightOverflowPx;

  const pillFarReservePx =
    Math.max(
      magnifyHeightOverflowPx,
      TOOLTIP_GAP_PX + TOOLTIP_HEIGHT_PX,
      TOOLTIP_GAP_PX + CONTEXT_MENU_HEIGHT_PX,
    ) - dockPaddingYPx;

  const windowThicknessDip = DOCK_EDGE_INSET_PX + pillThicknessPx + pillFarReservePx;

  return {
    iconSizePx,
    iconCornerRadiusPx: iconSizePx * ICON_CORNER_RADIUS_RATIO,
    dockGapPx,
    dockPaddingXPx,
    dockPaddingYPx,
    iconLedGapPx,
    magnifyHeightOverflowPx,
    magnifyInfluenceRadiusPx,
    launchBounceAmplitudePx,
    pillThicknessPx,
    pillThicknessHoverPx,
    pillFarReservePx,
    windowThicknessDip,
  };
}

/**
 * Pill size at rest along the length axis (grows/shrinks with item count —
 * padding + icons + gaps), for `appCount` icons at the given icon size.
 * Maps onto CSS width for `dockPosition: "bottom"|"top"`, CSS height for
 * `"left"|"right"`. The CSS pill itself is content-driven on this axis (no
 * explicit size is ever set in JSX, flex sizes it from its children), so
 * nothing in the frontend actually calls this at runtime. It exists purely
 * as the documented formula that `pill_length_dip`/`window_length_dip` in
 * src-tauri/src/platform/macos.rs mirror for native window/vibrancy/
 * hit-test sizing — kept here, parametrized by count and icon size instead
 * of fixed constants, so the two sides stay provably in sync. The actual
 * on-screen pill's vibrancy blur mask is independently corrected from the
 * measured DOM rect at runtime (see `sync_vibrancy_pill` /
 * `commands/window.rs`) — this formula only has to get the *native window
 * frame* long enough to avoid clipping it.
 */
export function pillLengthPx(items: DockItem[], iconSizePx: number): number {
  const metrics = getSizeMetrics(iconSizePx);
  let rowLength = 0;
  items.forEach((item, index) => {
    if (index > 0) {
      rowLength += metrics.dockGapPx;
    }
    rowLength +=
      item.type === "app" ? iconSizePx : DOCK_SEPARATOR_WIDTH_PX;
  });
  return metrics.dockPaddingXPx * 2 + rowLength;
}

/** See `pillLengthPx` — same "documented formula, not called at runtime" note. */
export function windowLengthDip(items: DockItem[], iconSizePx: number): number {
  return (
    pillLengthPx(items, iconSizePx) +
    Math.ceil(iconSizePx * (MAGNIFY_MAX_SCALE - 1)) +
    WINDOW_GLOW_BLEED_PX
  );
}

/**
 * CSS animation engine for a background preset — ported from spotlight-app's
 * `RgbPresetAnimation`. Each preset picks one; the border flow-ring styles
 * reuse the same engines independently via `BORDER_STYLE_PRESETS`.
 */
export type BackgroundAnimation = "static" | "spin" | "spin-tri" | "sweep" | "pulse";

/** Maps a `BackgroundAnimation` to the CSS classes applied on `.dock-bg-flow`
 * in DockPanel — kept here so settings UI never needs CSS implementation
 * details. */
export const BG_ANIMATION_CLASSES: Record<BackgroundAnimation, string> = {
  static: "dock-bg-anim-static",
  spin: "dock-bg-anim-spin animate-rgb-spin",
  "spin-tri": "dock-bg-anim-spin-tri animate-rgb-spin",
  sweep: "dock-bg-anim-sweep animate-rgb-sweep",
  pulse: "dock-bg-anim-pulse animate-rgb-spin animate-neon-pulse",
};

/** Spotlight-style 3-stop palette expanded to 6 for `rgbGlowColors` /
 * `--dock-bg-*` custom-property convention. */
function tripleToSix(
  c1: string,
  c2: string,
  c3: string,
): [string, string, string, string, string, string] {
  return [c1, c2, c3, c1, c2, c3];
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
  /** Which CSS animation engine drives this preset's flow layer. */
  animation: BackgroundAnimation;
  /** Linear-gradient angle in degrees (sweep/static) or conic base angle. */
  angle: number;
  /** Cycle duration in seconds at `backgroundSpeed` = 0.5; 0 for static. */
  baseDuration: number;
}

export const BACKGROUND_PRESETS: BackgroundPreset[] = [
  {
    id: "chroma",
    label: "Chroma",
    colors: ["#ff3b6b", "#ff9d3b", "#e9ff3b", "#3bffb0", "#3bb0ff", "#b03bff"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
  {
    id: "static",
    label: "Статичный",
    colors: tripleToSix("#7928ca", "#7928ca", "#7928ca"),
    animation: "static",
    angle: 135,
    baseDuration: 0,
  },
  {
    id: "two-color",
    label: "Двухцветный",
    colors: tripleToSix("#00f0ff", "#ff00aa", "#ff00aa"),
    animation: "spin",
    angle: 135,
    baseDuration: 4,
  },
  {
    id: "rainbow",
    label: "Радуга",
    colors: tripleToSix("#ff0080", "#7928ca", "#0070f3"),
    animation: "sweep",
    angle: 90,
    baseDuration: 4,
  },
  {
    id: "cyberpunk",
    label: "Cyberpunk",
    colors: tripleToSix("#00f0ff", "#ff00aa", "#ffe600"),
    animation: "spin-tri",
    angle: 135,
    baseDuration: 3,
  },
  {
    id: "toxic",
    label: "Toxic",
    colors: tripleToSix("#39ff14", "#00ff88", "#b8ff00"),
    animation: "spin-tri",
    angle: 90,
    baseDuration: 2.5,
  },
  {
    id: "inferno",
    label: "Inferno",
    colors: ["#ff003c", "#ff6a00", "#ffb700", "#ff2e00", "#ff8a00", "#ff003c"],
    animation: "sweep",
    angle: 45,
    baseDuration: 14,
  },
  {
    id: "sunset",
    label: "Закат",
    colors: tripleToSix("#ff6b35", "#f72585", "#7209b7"),
    animation: "sweep",
    angle: 120,
    baseDuration: 5,
  },
  {
    id: "ocean",
    label: "Океан",
    colors: tripleToSix("#00d4ff", "#0099ff", "#7b2ff7"),
    animation: "sweep",
    angle: 160,
    baseDuration: 4,
  },
  {
    id: "lava",
    label: "Лава",
    colors: tripleToSix("#ff4500", "#ff006e", "#ffbe0b"),
    animation: "sweep",
    angle: 45,
    baseDuration: 3.5,
  },
  {
    id: "aurora",
    label: "Aurora",
    colors: tripleToSix("#00ff87", "#60efff", "#a855f7"),
    animation: "sweep",
    angle: 100,
    baseDuration: 6,
  },
  {
    id: "synthwave",
    label: "Синтвейв",
    colors: tripleToSix("#ff00de", "#7b2ff7", "#00f5ff"),
    animation: "spin-tri",
    angle: 180,
    baseDuration: 4,
  },
  {
    id: "neon-pulse",
    label: "Неон-пульс",
    colors: tripleToSix("#ff006e", "#8338ec", "#3a86ff"),
    animation: "pulse",
    angle: 180,
    baseDuration: 2,
  },
  {
    id: "frost",
    label: "Frost",
    colors: ["#0072ff", "#00c6ff", "#7de2fc", "#c2f5ff", "#66d9ff", "#0072ff"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
  {
    id: "vapor",
    label: "Vaporwave",
    colors: ["#ff71ce", "#b967ff", "#01cdfe", "#05ffa1", "#fffb96", "#ff71ce"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
  {
    id: "matrix",
    label: "Matrix",
    colors: ["#003b00", "#008f11", "#00ff41", "#39ff14", "#00ff41", "#008f11"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
  {
    id: "plasma",
    label: "Plasma",
    colors: ["#7b2ff7", "#f107a3", "#ff6b6b", "#feca57", "#f107a3", "#7b2ff7"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
  {
    id: "bloodmoon",
    label: "Blood Moon",
    colors: ["#1a0000", "#8b0000", "#ff0000", "#ff4500", "#8b0000", "#1a0000"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
  {
    id: "neon-noir",
    label: "Neon Noir",
    colors: ["#ff00ff", "#8000ff", "#0080ff", "#ff00aa", "#4b0082", "#ff00ff"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
  {
    id: "solar",
    label: "Solar Flare",
    colors: ["#ffd700", "#ffae00", "#ff7b00", "#fff2cc", "#ffae00", "#ffd700"],
    animation: "sweep",
    angle: 100,
    baseDuration: 14,
  },
];

/** Falls back to the first preset for an unrecognized id — e.g. a config
 * file written by a future version with a preset this build doesn't know
 * about yet, rather than rendering nothing. */
export function getBackgroundPreset(id: string): BackgroundPreset {
  return BACKGROUND_PRESETS.find((preset) => preset.id === id) ?? BACKGROUND_PRESETS[0];
}

/** Per-preset duration derived from spotlight's `speedToDuration`, mapped
 * onto the dock's 0..1 `backgroundSpeed` slider — at speed 0.5 the cycle
 * equals `preset.baseDuration`; faster speeds shorten it proportionally. */
export function backgroundPresetToDurationS(
  preset: BackgroundPreset,
  speed: number,
): number {
  if (preset.animation === "static" || preset.baseDuration === 0) return 0;
  const clampedSpeed = Math.max(0.05, Math.min(1, speed));
  return preset.baseDuration / (clampedSpeed / 0.5);
}

/** Variant of the gradient flow-ring overlay for spotlight-style border
 * styles — separate from `BackgroundAnimation` because border and background
 * presets are chosen independently in dock settings. */
export type FlowRingVariant = "static" | "spin" | "spin-tri" | "sweep" | "pulse";

/** CSS classes for `.dock-border-flow-ring` per variant — mirrors
 * `BG_ANIMATION_CLASSES` but reads `--dock-glow-*` instead of `--dock-bg-*`. */
export const FLOW_RING_ANIMATION_CLASSES: Record<FlowRingVariant, string> = {
  static: "dock-border-flow-static",
  spin: "dock-border-flow-spin animate-rgb-spin",
  "spin-tri": "dock-border-flow-spin-tri animate-rgb-spin",
  sweep: "dock-border-flow-sweep animate-rgb-sweep",
  pulse: "dock-border-flow-pulse animate-rgb-spin animate-neon-pulse",
};

const FLOW_RING_VARIANT_BY_BORDER_STYLE: Record<string, FlowRingVariant> = {
  "flow-static": "static",
  "flow-spin": "spin",
  "flow-spin-tri": "spin-tri",
  "flow-sweep": "sweep",
  "flow-neon-pulse": "pulse",
};

/** Returns the flow-ring variant for spotlight-style border styles, or
 * `null` for styles that use border-color keyframes / scan ring instead. */
export function getFlowRingVariant(borderStyleId: string): FlowRingVariant | null {
  return FLOW_RING_VARIANT_BY_BORDER_STYLE[borderStyleId] ?? null;
}

/** CSS classes for the unified `.dock-border-ring` overlay per border style. */
export const BORDER_RING_CLASSES: Record<string, string> = {
  spectrum:
    "dock-border-ring dock-border-spectrum animate-border-spectrum-overlay",
  pulse: "dock-border-ring dock-border-pulse animate-border-pulse-overlay",
  glitch: "dock-border-ring dock-border-glitch animate-border-glitch-overlay",
  scan: "dock-border-ring dock-border-scan-ring animate-border-scan-rotate",
  "flow-sweep":
    "dock-border-ring dock-border-flow-ring dock-border-flow-sweep animate-border-flow-ring",
  "flow-spin":
    "dock-border-ring dock-border-flow-ring dock-border-flow-spin animate-border-flow-ring",
  "flow-spin-tri":
    "dock-border-ring dock-border-flow-ring dock-border-flow-spin-tri animate-border-flow-ring",
  "flow-neon-pulse":
    "dock-border-ring dock-border-flow-ring dock-border-flow-pulse animate-border-flow-ring animate-neon-pulse",
  "flow-static":
    "dock-border-ring dock-border-flow-ring dock-border-flow-static",
};

export function getBorderRingClasses(borderStyleId: string): string {
  return BORDER_RING_CLASSES[borderStyleId] ?? BORDER_RING_CLASSES.spectrum;
}

/**
 * A cyberpunk-styled animation driving the pill's RGB frame while
 * `DockSettings.animationsEnabled` is on — picked by id in
 * `DockSettings.borderStyle`. `animationClass` names a Tailwind
 * `--animate-*` utility registered in `src/index.css`; `"scan"` and the
 * `flow-*` styles are exceptions — they render no border-color keyframe
 * of their own and instead get a dedicated gradient ring overlay in
 * `DockPanel.tsx` (see `dock-border-scan-ring` / `dock-border-flow-ring`
 * in index.css), so their `animationClass` is left empty.
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
    description: "Яркая бегущая линия с хвостом, вращается по периметру рамки.",
    animationClass: "",
  },
  {
    id: "flow-sweep",
    label: "Поток",
    description: "Линейный градиент, плавно текущий по периметру рамки.",
    animationClass: "",
  },
  {
    id: "flow-spin",
    label: "Вращение",
    description: "Двухцветный конический градиент, вращающийся по рамке.",
    animationClass: "",
  },
  {
    id: "flow-spin-tri",
    label: "Трёхцветное",
    description: "Трёхцветный конический градиент, вращающийся по рамке.",
    animationClass: "",
  },
  {
    id: "flow-neon-pulse",
    label: "Неон-пульс",
    description: "Вращающийся градиент с пульсирующей яркостью неона.",
    animationClass: "",
  },
  {
    id: "flow-static",
    label: "Статичный",
    description: "Неподвижный градиент по периметру рамки.",
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
  borderWidthPx: DEFAULT_BORDER_WIDTH_PX,
  panelEffectEnabled: true,
  panelEffect: "grid",
  backgroundAnimationEnabled: true,
  backgroundPreset: "chroma",
  backgroundIntensity: 0.7,
  backgroundVisibility: 0.45,
  backgroundSpeed: 0.4,
  iconSizePreset: "medium",
  iconSizePx: DEFAULT_ICON_SIZE_PX,
  magnifyNeighborStrength: DEFAULT_MAGNIFY_NEIGHBOR_STRENGTH,
  ledColorMode: "auto",
  ledFixedColor: "#ff9d3b",
  dockPosition: "bottom",
};

export type LedColorMode = DockSettings["ledColorMode"];
export type DockPositionOption = DockSettings["dockPosition"];

export const DOCK_POSITION_OPTIONS: {
  id: DockPositionOption;
  label: string;
  description: string;
}[] = [
  {
    id: "bottom",
    label: "Снизу",
    description: "Горизонтальная панель у нижнего края экрана",
  },
  {
    id: "top",
    label: "Сверху",
    description: "Горизонтальная панель у верхнего края экрана",
  },
  {
    id: "left",
    label: "Слева",
    description: "Вертикальная панель у левого края экрана",
  },
  {
    id: "right",
    label: "Справа",
    description: "Вертикальная панель у правого края экрана",
  },
];

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
