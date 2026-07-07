import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  motion,
  useMotionValue,
  useSpring,
  useTransform,
  type MotionValue,
} from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FolderOpen, Power, Trash2 } from "lucide-react";
import type { DockApp } from "../lib/types";
import { MAGNIFY_INFLUENCE_RADIUS_PX, MAGNIFY_MAX_SCALE, TOOLTIP_GAP_PX } from "../lib/constants";

interface WindowLogicalPoint {
  x: number;
  y: number;
}

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
  /**
   * Bumped by `DockPanel` once per `Reorder.Item` whose layout box finished
   * animating into its post-reorder position. A reorder-drag starts and
   * ends while the cursor never leaves the pill, so it triggers neither
   * `ResizeObserver` (position, not size, changed) nor a new hover session
   * — this closes that gap the same way `hoverSessionId` closes the
   * layout-shift gap above, see the recalculation effect below.
   */
  reorderSettledId?: number;
  /** Right-click → "Remove from Dock" — mirrors the real macOS Dock pattern
   * (context menu item, not a separate "edit mode"). */
  onRemove?: (bundleId: string) => void;
  /** Right-click → "Show in Finder" — always available, regardless of
   * running state (just reveals the installed `.app`). */
  onShowInFinder?: (bundleId: string) => void;
  /** Right-click → "Quit" — only rendered when `app.isActive`; see the menu
   * JSX below for why this is distinct from `onRemove`. */
  onQuit?: (bundleId: string) => void;
  /** While an icon is being drag-reordered, magnify is suppressed. */
  isDragging?: boolean;
  /** Mirrors `DockSettings.animationsEnabled` — gates only the LED's
   * "breathing" pulse keyframe, not its on/off (active/inactive) state,
   * which is a functional signal, not decoration. */
  animationsEnabled?: boolean;
}

export function DockIcon({
  app,
  registerRef,
  mouseX,
  isHovered = false,
  hoverSessionId = 0,
  reorderSettledId = 0,
  onRemove,
  onShowInFinder,
  onQuit,
  isDragging = false,
  animationsEnabled = true,
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
  //
  // This `window` listener only ever sees clicks that land inside this
  // window's own web content — under the dock's click-through design
  // (`platform::macos::start_click_through_poller`) that's a small minority
  // of the screen. It's kept as a same-window fast path; the
  // `dock-global-mousedown` listener below is what actually makes "click
  // anywhere else" (other apps, the desktop) close the menu like a real
  // macOS one.
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

  // Global click-anywhere-closes-it, the way a real `NSMenu` behaves.
  // `dock-global-mousedown` is emitted by a HID-level event tap on the Rust
  // side for every left-mouse-down on screen, regardless of this window's
  // own click-through state — see `start_dock_click_tap` in
  // `platform/macos.rs`. Coordinates arrive already converted to this
  // window's logical space, so they compare directly against the menu's own
  // `getBoundingClientRect()` with no further conversion.
  useEffect(() => {
    if (!menuOpen) return;

    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void listen<WindowLogicalPoint>("dock-global-mousedown", (event) => {
      const rect = menuRef.current?.getBoundingClientRect();
      if (!rect) return;
      const { x, y } = event.payload;
      const insideMenu =
        x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
      if (!insideMenu) setMenuOpen(false);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [menuOpen]);

  // Reports the open menu's real footprint to the Rust side so the native
  // click-through hit-test can grow to cover it — see
  // `AppsState::menu_overlay_height_dip`. Without this, only the bottommost
  // menu row (closest to the icon) falls inside the fixed magnify-overflow
  // band the hit-test otherwise uses; anything above that is silently
  // click-through, which is what made the menu look "frozen" once the
  // cursor moved up into it. `useLayoutEffect`, not `useEffect`, so the
  // measurement happens before paint — the menu is already in the DOM by
  // the time this runs (mounted by the `menuOpen &&` guard below in the
  // same render), so there's no extra frame where it's visible but not yet
  // reachable.
  useLayoutEffect(() => {
    const reportMenuOverlay = (active: boolean, height: number) => {
      invoke("set_menu_overlay", { active, height }).catch((error: unknown) => {
        console.error("Failed to sync menu overlay hit-test region:", error);
      });
    };

    if (!menuOpen) {
      reportMenuOverlay(false, 0);
      return;
    }
    reportMenuOverlay(true, menuRef.current?.getBoundingClientRect().height ?? 0);
    return () => reportMenuOverlay(false, 0);
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
    if (!el || (hoverSessionId === 0 && reorderSettledId === 0)) return;
    const rect = el.getBoundingClientRect();
    centerX.set(rect.left + rect.width / 2);
  }, [hoverSessionId, reorderSettledId, centerX]);

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
                onShowInFinder?.(app.bundleId);
              }}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-zinc-800"
            >
              <FolderOpen className="h-3.5 w-3.5" />
              Show in Finder
            </button>

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

            {/* Quit terminates the running process (soft `terminate()`) and
                leaves the dock list untouched — the opposite of Remove,
                which edits the dock list and never touches the process. Kept
                alone at the bottom (after a divider) to mirror the real
                macOS Dock, where Quit is always the last item. */}
            {app.isActive && (
              <>
                <div className="h-px bg-zinc-700/70" />

                <button
                  type="button"
                  onClick={() => {
                    setMenuOpen(false);
                    onQuit?.(app.bundleId);
                  }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-zinc-800"
                >
                  <Power className="h-3.5 w-3.5" />
                  Quit
                </button>
              </>
            )}
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
            ? `${app.color} opacity-100 ${
                animationsEnabled
                  ? "animate-led-pulse"
                  : "shadow-[0_0_10px_2px_currentColor]"
              }`
            : "scale-0 text-transparent opacity-0"
        }`}
      />
    </div>
  );
}
