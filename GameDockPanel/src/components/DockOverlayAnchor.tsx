import type { CSSProperties, ReactNode, Ref } from "react";
import {
  overlayAnchorClassName,
  overlayAnchorMarginStyle,
  type OverlaySide,
} from "../lib/dockPlacement";

interface DockOverlayAnchorProps {
  side: OverlaySide;
  gap: number;
  className?: string;
  style?: CSSProperties;
  innerRef?: Ref<HTMLDivElement>;
  children: ReactNode;
}

/** Positions tooltip/menu content on the chosen side of its anchor icon. */
export function DockOverlayAnchor({
  side,
  gap,
  className = "",
  style,
  innerRef,
  children,
}: DockOverlayAnchorProps) {
  return (
    <div
      ref={innerRef}
      style={{ ...overlayAnchorMarginStyle(side, gap), ...style }}
      className={`${overlayAnchorClassName(side)} ${className}`}
    >
      {children}
    </div>
  );
}
