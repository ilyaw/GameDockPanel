import {
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
  type Ref,
} from "react";
import {
  overlayAnchorClassName,
  overlayAnchorMarginStyle,
  resolveOverlayCrossAxisOffset,
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

function mergeRefs<T>(...refs: Array<Ref<T> | undefined>) {
  return (node: T | null) => {
    for (const ref of refs) {
      if (!ref) continue;
      if (typeof ref === "function") ref(node);
      else ref.current = node;
    }
  };
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
  const localRef = useRef<HTMLDivElement>(null);
  const [crossAxisOffset, setCrossAxisOffset] = useState(0);

  useLayoutEffect(() => {
    const el = localRef.current;
    const anchor = el?.parentElement;
    if (!el || !anchor) return;

    const measure = () => {
      const anchorRect = anchor.getBoundingClientRect();
      const overlayRect = el.getBoundingClientRect();
      setCrossAxisOffset(
        resolveOverlayCrossAxisOffset(anchorRect, overlayRect, side),
      );
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [side, children]);

  return (
    <div
      ref={mergeRefs(localRef, innerRef)}
      style={{
        ...overlayAnchorMarginStyle(side, gap, crossAxisOffset),
        ...style,
      }}
      className={`${overlayAnchorClassName(side)} ${className}`}
    >
      {children}
    </div>
  );
}
