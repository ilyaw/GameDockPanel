import type { CSSProperties } from "react";

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
): CSSProperties {
  switch (side) {
    case "top":
      return { marginBottom: gap };
    case "bottom":
      return { marginTop: gap };
    case "left":
      return { marginRight: gap };
    case "right":
      return { marginLeft: gap };
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
