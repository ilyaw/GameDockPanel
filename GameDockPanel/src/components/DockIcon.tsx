import { useRef, useState } from "react";
import {
  motion,
  useSpring,
  useTransform,
  type MotionValue,
} from "framer-motion";
import type { DockApp } from "../lib/types";
import { MAGNIFY_INFLUENCE_RADIUS_PX, MAGNIFY_MAX_SCALE } from "../lib/constants";

/** Snappy, near-critically-damped feel — matches macOS Dock's minimal bounce. */
const MAGNIFY_SPRING = { mass: 0.15, stiffness: 300, damping: 25 };

interface DockIconProps {
  app: DockApp;
  registerRef?: (id: string, el: HTMLButtonElement | null) => void;
  /** Cursor X in viewport coords — see DockPanel's onMouseMove. Drives the
   * continuous magnify curve below; Infinity (cursor outside the pill)
   * clamps every icon back to rest scale. */
  mouseX: MotionValue<number>;
  /** Exact icon under the cursor (nearest-center hit test) — gates the
   * tooltip + stacking order only, not the magnify scale itself. */
  isHovered?: boolean;
}

export function DockIcon({
  app,
  registerRef,
  mouseX,
  isHovered = false,
}: DockIconProps) {
  const [broken, setBroken] = useState(false);
  const ref = useRef<HTMLButtonElement | null>(null);

  // Continuous macOS-style falloff: peak MAGNIFY_MAX_SCALE at this icon's
  // own center, easing back to rest (1) by MAGNIFY_INFLUENCE_RADIUS_PX away.
  // transform-only (no width/margin) so neighbors never reflow — the window
  // is already sized with MAGNIFY_MAX_SCALE headroom for exactly this (see
  // constants.ts + tauri-glass-dock skill).
  const distance = useTransform(mouseX, (val) => {
    const bounds = ref.current?.getBoundingClientRect();
    if (!bounds) return Infinity;
    return val - (bounds.left + bounds.width / 2);
  });
  const scaleRaw = useTransform(
    distance,
    [-MAGNIFY_INFLUENCE_RADIUS_PX, 0, MAGNIFY_INFLUENCE_RADIUS_PX],
    [1, MAGNIFY_MAX_SCALE, 1],
  );
  const scale = useSpring(scaleRaw, MAGNIFY_SPRING);

  const iconVisualClass =
    "h-14 w-14 origin-bottom rounded-2xl shadow-lg shadow-black/40";

  return (
    <button
      type="button"
      ref={(el) => {
        ref.current = el;
        registerRef?.(app.id, el);
      }}
      aria-pressed={app.isActive}
      aria-label={`${app.name}${app.isActive ? " (running)" : ""}`}
      className={`relative flex flex-col items-center gap-2 outline-none ${
        isHovered ? "z-10" : "z-0"
      }`}
    >
      <div className="relative shrink-0">
        <span
          className={`pointer-events-none absolute bottom-full left-1/2 z-20 mb-0.5 -translate-x-1/2 whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 shadow-lg shadow-black/40 transition-all duration-300 ease-out ${
            isHovered
              ? "scale-100 opacity-100"
              : "scale-90 opacity-0"
          }`}
        >
          {app.name}
        </span>

        {broken ? (
          <motion.div
            style={{ scale }}
            className={`flex items-center justify-center bg-zinc-800 text-lg font-semibold ${iconVisualClass} ${app.color}`}
          >
            {app.name.slice(0, 2).toUpperCase()}
          </motion.div>
        ) : (
          <motion.img
            style={{ scale }}
            src={app.iconUrl}
            alt={app.name}
            draggable={false}
            onError={() => setBroken(true)}
            className={`object-cover ${iconVisualClass}`}
          />
        )}
      </div>

      <span
        className={`h-[3px] w-6 rounded-full bg-current transition-all duration-300 ease-out ${
          app.isActive
            ? `${app.color} animate-led-pulse opacity-100`
            : "scale-0 text-transparent opacity-0"
        }`}
      />
    </button>
  );
}
