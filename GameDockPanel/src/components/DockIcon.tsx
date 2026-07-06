import { useLayoutEffect, useRef, useState } from "react";
import {
  motion,
  useMotionValue,
  useSpring,
  useTransform,
  type MotionValue,
} from "framer-motion";
import type { DockApp } from "../lib/types";
import { MAGNIFY_INFLUENCE_RADIUS_PX, MAGNIFY_MAX_SCALE, TOOLTIP_GAP_PX } from "../lib/constants";

const MAGNIFY_SPRING = { mass: 0.15, stiffness: 300, damping: 25 };

interface DockIconProps {
  app: DockApp;
  /** Scaled visual node — hit-test uses its transformed bounding box at click time. */
  registerRef?: (id: string, el: HTMLElement | null) => void;
  mouseX: MotionValue<number>;
  isHovered?: boolean;
}

export function DockIcon({
  app,
  registerRef,
  mouseX,
  isHovered = false,
}: DockIconProps) {
  const [broken, setBroken] = useState(false);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const centerX = useMotionValue(0);

  // Rest-layout center X for the magnify distance curve — re-measured on layout
  // shifts (DPI, icon count), not on every mousemove frame.
  useLayoutEffect(() => {
    const el = buttonRef.current;
    if (!el) return;

    const measure = () => {
      const rect = el.getBoundingClientRect();
      centerX.set(rect.left + rect.width / 2);
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [centerX]);

  const distance = useTransform([mouseX, centerX], ([mx, cx]: number[]) => {
    if (!Number.isFinite(mx)) return Infinity;
    return mx - cx;
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
      ref={buttonRef}
      aria-pressed={app.isActive}
      aria-label={`${app.name}${app.isActive ? " (running)" : ""}`}
      className={`relative flex flex-col items-center gap-2 outline-none ${
        isHovered ? "z-10" : "z-0"
      }`}
    >
      <div className="relative shrink-0">
        <span
          style={{ marginBottom: TOOLTIP_GAP_PX }}
          className={`pointer-events-none absolute bottom-full left-1/2 z-20 -translate-x-1/2 whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 shadow-lg shadow-black/40 transition-all duration-300 ease-out ${
            isHovered
              ? "scale-100 opacity-100"
              : "scale-90 opacity-0"
          }`}
        >
          {app.name}
        </span>

        {broken ? (
          <motion.div
            ref={(el) => registerRef?.(app.id, el)}
            style={{ scale }}
            className={`flex items-center justify-center bg-zinc-800 text-lg font-semibold ${iconVisualClass} ${app.color}`}
          >
            {app.name.slice(0, 2).toUpperCase()}
          </motion.div>
        ) : (
          <motion.img
            ref={(el) => registerRef?.(app.id, el)}
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
