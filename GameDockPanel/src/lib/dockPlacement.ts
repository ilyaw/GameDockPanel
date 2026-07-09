import type { CSSProperties } from "react";
import { DOCK_EDGE_INSET_PX } from "./constants";

/** Which viewport axis magnify distance is measured on. */
export type MagnifyAxis = "x" | "y";

/** CSS `transform-origin` keyword — icon grows toward the desktop center. */
export type MagnifyTransformOrigin = "top" | "bottom" | "left" | "right";

/** Side of the anchor where tooltip/menu is placed. */
export type OverlaySide = "top" | "bottom" | "left" | "right";

export const OPPOSITE_OVERLAY_SIDE: Record<OverlaySide, OverlaySide> = {
  top: "bottom",
  bottom: "top",
  left: "right",
  right: "left",
};

/**
 * Picks the overlay side with enough viewport room. Starts from
 * `preferred` (from dock orientation), flips to the opposite when space
 * is insufficient, then falls back to whichever side has more room.
 */
export function resolveOverlaySide(
  anchorRect: DOMRect,
  overlaySize: { width: number; height: number },
  preferred: OverlaySide,
  gap: number,
): OverlaySide {
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  const space: Record<OverlaySide, number> = {
    top: anchorRect.top,
    bottom: vh - anchorRect.bottom,
    left: anchorRect.left,
    right: vw - anchorRect.right,
  };

  const need: Record<OverlaySide, number> = {
    top: gap + overlaySize.height,
    bottom: gap + overlaySize.height,
    left: gap + overlaySize.width,
    right: gap + overlaySize.width,
  };

  if (space[preferred] >= need[preferred]) return preferred;

  const opposite = OPPOSITE_OVERLAY_SIDE[preferred];
  if (space[opposite] >= need[opposite]) return opposite;

  return space[preferred] >= space[opposite] ? preferred : opposite;
}

/**
 * Horizontal shift for top/bottom overlays (vertical for left/right) so a
 * center-anchored tooltip/menu stays inside the webview when the anchor sits
 * near a screen edge. Pure geometry — no readback from the overlay DOM.
 */
export function resolveOverlayCrossAxisOffset(
  anchorRect: DOMRect,
  overlaySize: { width: number; height: number },
  side: OverlaySide,
  viewport: { width: number; height: number } = {
    width: window.innerWidth,
    height: window.innerHeight,
  },
  padding = DOCK_EDGE_INSET_PX,
): number {
  if (overlaySideIsVertical(side)) {
    const centerX = anchorRect.left + anchorRect.width / 2;
    const halfW = overlaySize.width / 2;
    const left = centerX - halfW;
    const right = centerX + halfW;
    if (left < padding) return padding - left;
    if (right > viewport.width - padding) {
      return viewport.width - padding - right;
    }
    return 0;
  }

  const centerY = anchorRect.top + anchorRect.height / 2;
  const halfH = overlaySize.height / 2;
  const top = centerY - halfH;
  const bottom = centerY + halfH;
  if (top < padding) return padding - top;
  if (bottom > viewport.height - padding) {
    return viewport.height - padding - bottom;
  }
  return 0;
}

export function overlayAnchorClassName(side: OverlaySide, zIndex = "z-20"): string {
  const base = `absolute ${zIndex}`;
  switch (side) {
    case "top":
      return `${base} bottom-full left-1/2 -translate-x-1/2`;
    case "bottom":
      return `${base} top-full left-1/2 -translate-x-1/2`;
    case "left":
      return `${base} right-full top-1/2 -translate-y-1/2`;
    case "right":
      return `${base} left-full top-1/2 -translate-y-1/2`;
  }
}

export function overlayAnchorMarginStyle(
  side: OverlaySide,
  gap: number,
  crossAxisOffset = 0,
): CSSProperties {
  switch (side) {
    case "top":
      return { marginBottom: gap, marginLeft: crossAxisOffset };
    case "bottom":
      return { marginTop: gap, marginLeft: crossAxisOffset };
    case "left":
      return { marginRight: gap, marginTop: crossAxisOffset };
    case "right":
      return { marginLeft: gap, marginTop: crossAxisOffset };
  }
}

export function magnifyOriginClassName(origin: MagnifyTransformOrigin): string {
  return `origin-${origin}`;
}

/** Rest-layout center on the magnify main axis (viewport coords). */
export function measureMagnifyCenter(rect: DOMRect, axis: MagnifyAxis): number {
  return axis === "x" ? rect.left + rect.width / 2 : rect.top + rect.height / 2;
}

/** Whether the overlay opens along the viewport Y axis (top/bottom). */
export function overlaySideIsVertical(side: OverlaySide): boolean {
  return side === "top" || side === "bottom";
}
