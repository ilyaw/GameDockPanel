import { useMemo } from "react";
import type {
  MagnifyAxis,
  MagnifyTransformOrigin,
  OverlaySide,
} from "../lib/dockPlacement";
import type { DockPosition } from "../lib/types";

/** Bar orientation for the running-app LED dot. */
export type LedAxis = "horizontal" | "vertical";

/**
 * Single source of truth for how `DockPosition` maps onto layout — Phase 1
 * of the positioning initiative (see PROMPT_15_POSITION_PHASE1.md).
 * Phase 2 adds magnify axis, transform-origin, and overlay preferred side
 * so tooltip/menu/magnify share one orientation model.
 */
export interface DockOrientation {
  position: DockPosition;
  /** True for `left`/`right` — icons stack in a column instead of a row. */
  isVertical: boolean;
  /** `Reorder.Group`'s own drag axis — it already supports both out of the
   * box, no custom reorder logic needed for the vertical case. */
  reorderAxis: "x" | "y";
  /** Viewport axis for magnify distance (`x` = horizontal docks, `y` = vertical). */
  magnifyAxis: MagnifyAxis;
  /** Icon scale grows toward the desktop center, away from the anchored edge. */
  magnifyTransformOrigin: MagnifyTransformOrigin;
  /** Default tooltip/menu side before viewport collision flip. */
  overlayPreferredSide: OverlaySide;
  /** `horizontal` — bar under the icon (`bottom`/`top`); `vertical` — bar
   * beside the icon toward the screen edge (`left`/`right`). */
  ledAxis: LedAxis;
  /** When `ledAxis` is `vertical`, render the LED before the icon (`left`
   * dock) or after it (`right` dock). Ignored for horizontal docks. */
  ledBeforeIcon: boolean;
  /**
   * Classes for the full-screen window-anchor wrapper: flex direction +
   * near-edge `justify-*` (which side the dock is pinned to) + `items-center`
   * (centers the content-sized pill on the cross axis — replaces the pill's
   * old `mx-auto`, which only ever centered on X) + the near-edge inset
   * padding (mirrors `DOCK_EDGE_INSET_PX`).
   */
  wrapperClassName: string;
  /**
   * Classes for the pill's own three direct children (icon list, divider,
   * icon row): main-axis direction + cross-axis alignment. `items-end`
   * bottom-aligns each child's row (icon sits above its LED dot) — still
   * correct for `top` (the row itself stays horizontal, only the whole
   * dock's screen position flips) so only `left`/`right` need `items-center`.
   */
  pillClassName: string;
}

/**
 * `isVertical` also doubles as "which CSS dimension gets the pill's
 * explicit, motion-driven thickness value" (`width` for `true`, `height`
 * for `false` — the other dimension stays content-driven). Consumers
 * should assign *both* `width` and `height` every render (falling back to
 * `"auto"`/`0` on the non-thickness one, never `undefined`) — Framer
 * Motion's imperative style application can leave a stale pixel value
 * stuck on a style key that silently disappears from the `style` object
 * between renders, which is exactly what happens across the `bottom` →
 * hydrated `DockPosition` transition on first mount if the other key is
 * simply omitted instead of explicitly reset. See `DockPanel`'s pill
 * `style` for the pattern.
 */
const ORIENTATION_BY_POSITION: Record<DockPosition, Omit<DockOrientation, "position">> = {
  bottom: {
    isVertical: false,
    reorderAxis: "x",
    magnifyAxis: "x",
    magnifyTransformOrigin: "bottom",
    overlayPreferredSide: "top",
    ledAxis: "horizontal",
    ledBeforeIcon: false,
    wrapperClassName: "flex-col justify-end items-center pb-2",
    pillClassName: "flex-row items-end",
  },
  top: {
    isVertical: false,
    reorderAxis: "x",
    magnifyAxis: "x",
    magnifyTransformOrigin: "top",
    overlayPreferredSide: "bottom",
    ledAxis: "horizontal",
    ledBeforeIcon: false,
    wrapperClassName: "flex-col justify-start items-center pt-2",
    pillClassName: "flex-row items-end",
  },
  left: {
    isVertical: true,
    reorderAxis: "y",
    magnifyAxis: "y",
    magnifyTransformOrigin: "left",
    overlayPreferredSide: "right",
    ledAxis: "vertical",
    ledBeforeIcon: true,
    wrapperClassName: "flex-row justify-start items-center pl-2",
    pillClassName: "flex-col items-center",
  },
  right: {
    isVertical: true,
    reorderAxis: "y",
    magnifyAxis: "y",
    magnifyTransformOrigin: "right",
    overlayPreferredSide: "left",
    ledAxis: "vertical",
    ledBeforeIcon: false,
    wrapperClassName: "flex-row justify-end items-center pr-2",
    pillClassName: "flex-col items-center",
  },
};

export function useDockOrientation(position: DockPosition): DockOrientation {
  return useMemo(
    () => ({ position, ...ORIENTATION_BY_POSITION[position] }),
    [position],
  );
}
