import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  motion,
  useMotionValue,
  useSpring,
  useTransform,
  type MotionValue,
} from "framer-motion";
import { Trash2 } from "lucide-react";
import type { DockApp } from "../lib/types";
import { MAGNIFY_INFLUENCE_RADIUS_PX, MAGNIFY_MAX_SCALE, TOOLTIP_GAP_PX } from "../lib/constants";

const MAGNIFY_SPRING = { mass: 0.15, stiffness: 300, damping: 25 };

interface DockIconProps {
  app: DockApp;
  /** Scaled visual node — hit-test uses its transformed bounding box at click time. */
  registerRef?: (id: string, el: HTMLElement | null) => void;
  mouseX: MotionValue<number>;
  isHovered?: boolean;
  /**
   * Bumped by `DockPanel` at the start of every hover session (pill
   * `mouseenter` / `dock-hover` → true). `ResizeObserver` alone only reacts
   * to size changes, not position shifts (e.g. `PILL_TOP_RESERVE` moving
   * while icon size stays the same) — re-measuring on hover-enter closes
   * that gap cheaply, without adding a `mousemove`-frequency cost.
   */
  hoverSessionId?: number;
  /** Right-click → "Remove from Dock" — mirrors the real macOS Dock pattern
   * (context menu item, not a separate "edit mode"). */
  onRemove?: (bundleId: string) => void;
  /** While an icon is being drag-reordered, magnify is suppressed. */
  isDragging?: boolean;
}

export function DockIcon({
  app,
  registerRef,
  mouseX,
  isHovered = false,
  hoverSessionId = 0,
  onRemove,
  isDragging = false,
}: DockIconProps) {
  const [broken, setBroken] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const buttonRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const centerX = useMotionValue(0);

  useEffect(() => {
    setBroken(false);
  }, [app.iconUrl]);

  // Close the context menu on an outside click or Escape. Checks
  // `menuRef.current?.contains(...)` rather than closing unconditionally on
  // every `mousedown` — otherwise the menu item's own click never lands: the
  // mousedown that starts that click would close (and unmount) the menu
  // before the subsequent click event fires on it.
  useEffect(() => {
    if (!menuOpen) return;

    const handlePointerDown = (event: MouseEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      setMenuOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [menuOpen]);

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

  useLayoutEffect(() => {
    const el = buttonRef.current;
    if (!el || hoverSessionId === 0) return;
    const rect = el.getBoundingClientRect();
    centerX.set(rect.left + rect.width / 2);
  }, [hoverSessionId, centerX]);

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

  // `null` means Rust couldn't resolve a native icon (app not installed) —
  // same fallback badge as a broken/failed `<img>` load.
  const showFallback = broken || !app.iconUrl;

  return (
    // A `<div>`, not `<button>`: it now has to contain the "Remove from
    // Dock" menu's own real `<button>`, and nested `<button>`s are invalid
    // HTML. No functionality is lost — activation has never gone through
    // this element's own click handler anyway (see the native
    // `dock-click`/`hitTestIcon` dispatch in DockPanel.tsx), so `role`/
    // `aria-*` here are purely there to keep the same assistive-tech
    // semantics `<button>` had.
    <div
      role="button"
      tabIndex={0}
      ref={buttonRef}
      aria-pressed={app.isActive}
      aria-label={`${app.name}${app.isActive ? " (running)" : ""}`}
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
      className={`relative flex flex-col items-center gap-2 outline-none ${
        isDragging ? "cursor-grabbing" : "cursor-grab"
      } ${isHovered || menuOpen ? "z-10" : "z-0"}`}
    >
      <div className="relative shrink-0">
        <span
          style={{ marginBottom: TOOLTIP_GAP_PX }}
          className={`pointer-events-none absolute bottom-full left-1/2 z-20 -translate-x-1/2 whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 shadow-lg shadow-black/40 transition-all duration-300 ease-out ${
            isHovered && !menuOpen
              ? "scale-100 opacity-100"
              : "scale-90 opacity-0"
          }`}
        >
          {app.name}
        </span>

        {menuOpen && (
          <div
            ref={menuRef}
            style={{ marginBottom: TOOLTIP_GAP_PX }}
            className="pointer-events-auto absolute bottom-full left-1/2 z-30 -translate-x-1/2 overflow-hidden whitespace-nowrap rounded-md bg-zinc-900/95 text-xs text-zinc-200 shadow-lg shadow-black/40"
          >
            <button
              type="button"
              onClick={() => {
                setMenuOpen(false);
                onRemove?.(app.bundleId);
              }}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-zinc-800"
            >
              <Trash2 className="h-3.5 w-3.5" />
              Remove from Dock
            </button>
          </div>
        )}

        {showFallback ? (
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
            src={app.iconUrl ?? undefined}
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
    </div>
  );
}
