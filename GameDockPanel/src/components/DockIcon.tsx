import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  animate,
  motion,
  useMotionValue,
  useSpring,
  useTransform,
  type MotionValue,
} from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FolderOpen, Minus, Power, Trash2 } from "lucide-react";
import type { DockApp } from "../lib/types";
import {
  ICON_CORNER_RADIUS_RATIO,
  LED_EDGE_DOT_PX,
  LED_EDGE_INSET_FROM_PILL_PX,
  MAGNIFY_MAX_SCALE,
  TOOLTIP_GAP_PX,
  getSizeMetrics,
} from "../lib/constants";
import type { LedAxis } from "../hooks/useDockOrientation";
import {
  magnifyOriginClassName,
  measureMagnifyCenter,
  resolveOverlaySide,
  type MagnifyAxis,
  type MagnifyTransformOrigin,
  type OverlaySide,
} from "../lib/dockPlacement";
import { DockOverlayAnchor } from "./DockOverlayAnchor";

interface WindowLogicalPoint {
  x: number;
  y: number;
}

const MAGNIFY_SPRING = { mass: 0.15, stiffness: 300, damping: 25 };

/**
 * Launch-bounce keyframe shape — 4 decaying hops (relative to
 * `launchBounceAmplitudePx`) rather than one repeated fixed-height jump, per
 * the "decaying amplitude, not infinitely-equal jumps" requirement. Each
 * pair of segments is an up (`easeOut`, decelerating against "gravity") then
 * a down (`easeIn`, accelerating) — the last hop settles back to rest before
 * `LAUNCH_BOUNCE_REPEAT_DELAY_S` pauses and the whole decaying burst
 * restarts, so the icon keeps visibly bouncing for as long as the wait
 * lasts instead of freezing once the amplitude decays to zero.
 */
const LAUNCH_BOUNCE_RATIOS = [0, -1, 0, -0.62, 0, -0.38, 0, -0.2, 0];
const LAUNCH_BOUNCE_TIMES = [0, 0.11, 0.22, 0.32, 0.42, 0.53, 0.65, 0.8, 1];
const LAUNCH_BOUNCE_EASES = [
  "easeOut",
  "easeIn",
  "easeOut",
  "easeIn",
  "easeOut",
  "easeIn",
  "easeOut",
  "easeIn",
] as const;
const LAUNCH_BOUNCE_BURST_DURATION_S = 1.1;
const LAUNCH_BOUNCE_REPEAT_DELAY_S = 0.35;
/** How quickly `bounceY` eases back to rest once the bounce is told to stop
 * (running-state arrived, or the timeout fired) — a short tween instead of
 * a hard `jump(0)` so the icon doesn't visibly snap mid-arc. */
const LAUNCH_BOUNCE_STOP_TWEEN_S = 0.15;

interface DockIconProps {
  app: DockApp;
  /** Spring-driven icon edge length — visual sizes derive from this via
   * `useTransform` so the dock can resize smoothly without React re-renders
   * on every animation frame. */
  iconSizePx: MotionValue<number>;
  /** Scaled visual node — hit-test uses its transformed bounding box at click time. */
  registerRef?: (id: string, el: HTMLElement | null) => void;
  mouseX: MotionValue<number>;
  mouseY: MotionValue<number>;
  magnifyAxis: MagnifyAxis;
  magnifyTransformOrigin: MagnifyTransformOrigin;
  overlayPreferredSide: OverlaySide;
  /** `horizontal` under the icon, `vertical` beside it toward the screen edge. */
  ledAxis?: LedAxis;
  /** When `ledAxis` is `vertical`, LED renders before the icon (`left` dock). */
  ledBeforeIcon?: boolean;
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
   * Bumped by `DockPanel` once after all post-reorder layout animations have
   * settled (debounced). A reorder-drag starts and ends while the cursor
   * never leaves the pill, so it triggers neither `ResizeObserver` (position,
   * not size, changed) nor a new hover session — this closes that gap the
   * same way `hoverSessionId` closes the layout-shift gap above, see the
   * recalculation effect below.
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
  /** Right-click → manual LED color override (`null` resets to auto). */
  onSetIndicatorColor?: (bundleId: string, color: string | null) => void;
  onInsertSeparatorBefore?: (bundleId: string) => void;
  onInsertSeparatorAfter?: (bundleId: string) => void;
  separatorsFull?: boolean;
  /** While an icon is being drag-reordered, magnify is suppressed. */
  isDragging?: boolean;
  /** Brief post-drop window while layout position animation settles. */
  isReorderSettling?: boolean;
  /** True while any dock context menu is open — magnify is suppressed. */
  contextMenuActive?: boolean;
  /** Notifies `DockPanel` when this icon's context menu opens or closes. */
  onContextMenuOpenChange?: (open: boolean) => void;
  /** Mirrors `DockSettings.animationsEnabled` — gates only the LED's
   * "breathing" pulse keyframe, not its on/off (active/inactive) state,
   * which is a functional signal, not decoration. */
  animationsEnabled?: boolean;
  /** True while waiting for this app's launch to be observed via
   * `apps-running-changed` (or the timeout fallback) — see `useDockApps`'s
   * `bouncingIds`. Drives the launch-bounce animation below. */
  isBouncing?: boolean;
}

export function DockIcon({
  app,
  iconSizePx,
  registerRef,
  mouseX,
  mouseY,
  magnifyAxis,
  magnifyTransformOrigin,
  overlayPreferredSide,
  ledAxis = "horizontal",
  ledBeforeIcon = false,
  isHovered = false,
  hoverSessionId = 0,
  reorderSettledId = 0,
  onRemove,
  onShowInFinder,
  onQuit,
  onSetIndicatorColor,
  onInsertSeparatorBefore,
  onInsertSeparatorAfter,
  separatorsFull = false,
  isDragging = false,
  isReorderSettling = false,
  contextMenuActive = false,
  onContextMenuOpenChange,
  animationsEnabled = true,
  isBouncing = false,
}: DockIconProps) {
  const [broken, setBroken] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const buttonRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const colorPickActiveRef = useRef(false);
  const centerMain = useMotionValue(0);
  const [menuSide, setMenuSide] = useState<OverlaySide>(overlayPreferredSide);
  const magnifyOriginClass = magnifyOriginClassName(magnifyTransformOrigin);

  useEffect(() => {
    onContextMenuOpenChange?.(menuOpen);
    return () => {
      if (menuOpen) onContextMenuOpenChange?.(false);
    };
  }, [menuOpen, onContextMenuOpenChange]);

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
      if (colorPickActiveRef.current) return;
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
      if (colorPickActiveRef.current) return;
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
    const reportMenuOverlay = (
      active: boolean,
      side: OverlaySide,
      width: number,
      height: number,
    ) => {
      invoke("set_menu_overlay", { active, side, width, height }).catch(
        (error: unknown) => {
          console.error("Failed to sync menu overlay hit-test region:", error);
        },
      );
    };

    if (!menuOpen) {
      colorPickActiveRef.current = false;
      reportMenuOverlay(false, overlayPreferredSide, 0, 0);
      return;
    }

    const measure = () => {
      const anchorEl = buttonRef.current;
      const menuEl = menuRef.current;
      if (!anchorEl || !menuEl) return;

      const anchorRect = anchorEl.getBoundingClientRect();
      const menuRect = menuEl.getBoundingClientRect();
      const resolvedSide = resolveOverlaySide(
        anchorRect,
        { width: menuRect.width, height: menuRect.height },
        overlayPreferredSide,
        TOOLTIP_GAP_PX,
      );
      setMenuSide(resolvedSide);
      reportMenuOverlay(true, resolvedSide, menuRect.width, menuRect.height);
    };

    measure();
    const menuEl = menuRef.current;
    if (!menuEl) return;
    const observer = new ResizeObserver(measure);
    observer.observe(menuEl);
    return () => {
      observer.disconnect();
      colorPickActiveRef.current = false;
      reportMenuOverlay(false, overlayPreferredSide, 0, 0);
    };
  }, [menuOpen, app.isActive, app.indicatorColorOverride, overlayPreferredSide]);

  // Rest-layout center on the magnify main axis — re-measured on layout shifts
  // (DPI, icon count), not on every mousemove frame.
  useLayoutEffect(() => {
    const el = buttonRef.current;
    if (!el) return;

    const measure = () => {
      const rect = el.getBoundingClientRect();
      centerMain.set(measureMagnifyCenter(rect, magnifyAxis));
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [centerMain, magnifyAxis]);

  useLayoutEffect(() => {
    const el = buttonRef.current;
    if (!el || (hoverSessionId === 0 && reorderSettledId === 0)) return;
    const rect = el.getBoundingClientRect();
    centerMain.set(measureMagnifyCenter(rect, magnifyAxis));
  }, [hoverSessionId, reorderSettledId, centerMain, magnifyAxis]);

  const magnifyInfluenceRadiusPx = useTransform(iconSizePx, (px) =>
    getSizeMetrics(px).magnifyInfluenceRadiusPx,
  );
  const scaleRaw = useTransform(
    [mouseX, mouseY, centerMain, magnifyInfluenceRadiusPx],
    ([mx, my, cm, radius]: number[]) => {
      const m = magnifyAxis === "x" ? mx : my;
      if (!Number.isFinite(m)) return 1;
      const distance = m - cm;
      const t = Math.abs(distance) / radius;
      if (t >= 1) return 1;
      return 1 + (MAGNIFY_MAX_SCALE - 1) * (1 - t);
    },
  );
  const scale = useSpring(scaleRaw, MAGNIFY_SPRING);
  const magnifySuppressed =
    isDragging || isReorderSettling || contextMenuActive;

  useLayoutEffect(() => {
    if (magnifySuppressed) {
      scale.jump(1);
    } else {
      scale.set(scaleRaw.get());
    }
  }, [magnifySuppressed, scale, scaleRaw]);

  /**
   * Independent motion value composed onto the same node as `scale` below —
   * Framer Motion recognizes `y`/`scale` as transform-shorthand style props
   * and always merges every one present into a single `transform` (see
   * `buildTransform` in `motion-dom`), in `translate` → `scale` order. That
   * means the bounce's `translateY` is always in unscaled px, unaffected by
   * whatever magnify scale the icon currently has — no suppression needed
   * between the two, unlike the reorder/magnify interaction above (which is
   * a different mechanism: `Reorder.Item`'s own layout-projection transform
   * on the *outer* wrapper, not a style prop on this inner node).
   *
   * Deliberately `translateY` on every `dockPosition`, not perpendicular to
   * the anchored edge (QA pass, PROMPT_17): the real macOS Dock's launch
   * bounce stays vertical even on a side dock, and the hop's amplitude
   * (`launchBounceAmplitudePx` ≈ the scaled padding) keeps the icon inside
   * the pill on all four positions at every icon size — checked at
   * 44/56/72. On `top` this means hopping toward the screen edge; accepted
   * as the price of "always vertical" rather than special-casing one edge.
   */
  const bounceY = useMotionValue(0);
  const bounceAmplitudePx = useTransform(
    iconSizePx,
    (px) => getSizeMetrics(px).launchBounceAmplitudePx,
  );
  /** Held so a bounce restart can cancel an in-flight stop tween on `bounceY`. */
  const bounceStopTweenRef = useRef<ReturnType<typeof animate> | null>(null);

  useEffect(() => {
    if (!isBouncing) return;

    bounceStopTweenRef.current?.stop();
    bounceStopTweenRef.current = null;

    const amplitude = bounceAmplitudePx.get();
    const controls = animate(
      bounceY,
      LAUNCH_BOUNCE_RATIOS.map((ratio) => ratio * amplitude),
      {
        times: LAUNCH_BOUNCE_TIMES,
        // Spread to a mutable copy — `LAUNCH_BOUNCE_EASES` is a `readonly`
        // `as const` tuple (needed so each entry keeps its literal
        // `Easing` name instead of widening to `string`), which the
        // `animate()` options type doesn't accept directly.
        ease: [...LAUNCH_BOUNCE_EASES],
        duration: LAUNCH_BOUNCE_BURST_DURATION_S,
        repeat: Infinity,
        repeatDelay: LAUNCH_BOUNCE_REPEAT_DELAY_S,
      },
    );

    return () => {
      controls.stop();
      bounceStopTweenRef.current?.stop();
      bounceStopTweenRef.current = animate(bounceY, 0, {
        duration: LAUNCH_BOUNCE_STOP_TWEEN_S,
        ease: "easeOut",
      });
    };
  }, [isBouncing, bounceY, bounceAmplitudePx]);

  const iconWidth = useTransform(iconSizePx, (px) => px);
  const iconHeight = useTransform(iconSizePx, (px) => px);
  const iconCornerRadius = useTransform(
    iconSizePx,
    (px) => px * ICON_CORNER_RADIUS_RATIO,
  );
  /** Icon↔LED gap — the same scaled metric the pill-thickness formula
   * (`getSizeMetrics().iconLedGapPx`) accounts for, not a fixed `gap-2`:
   * a fixed 8px drifts 1–3px against the formula at non-default sizes.
   * A `"<n>px"` string, not a number — `gap` is missing from Framer's
   * px-append map, so numeric post-mount updates are ignored by CSS (see
   * `pillGapPx` in DockPanel.tsx). */
  const iconLedGap = useTransform(
    iconSizePx,
    (px) => `${getSizeMetrics(px).iconLedGapPx}px`,
  );

  // `null` means Rust couldn't resolve a native icon (app not installed) —
  // same fallback badge as a broken/failed `<img>` load.
  const showFallback = broken || !app.iconUrl;

  // "Before/after in the item list" reads as left/right only while the dock
  // is a row — on a Left/Right dock the column makes it above/below. The
  // insert command itself is order-based and orientation-agnostic.
  const isColumnLayout = magnifyAxis === "y";
  const separatorBeforeLabel = isColumnLayout
    ? "Разделитель сверху"
    : "Разделитель слева";
  const separatorAfterLabel = isColumnLayout
    ? "Разделитель снизу"
    : "Разделитель справа";

  const edgeLedOffsetPx = useTransform(iconSizePx, (px) => {
    const pad = getSizeMetrics(px, { ledAlongThickness: false }).dockPaddingYPx;
    return Math.max(0, pad - LED_EDGE_INSET_FROM_PILL_PX - LED_EDGE_DOT_PX / 2);
  });
  const edgeLedInsetPx = useTransform(edgeLedOffsetPx, (offset) => -offset);

  const horizontalLedBar = (
    <span
      style={{ color: app.indicatorColor }}
      className={`h-[3px] w-6 shrink-0 rounded-full bg-current transition-all duration-300 ease-out ${
        app.isActive
          ? `opacity-100 ${
              animationsEnabled
                ? "animate-led-pulse"
                : "shadow-[0_0_10px_2px_currentColor]"
            }`
          : "scale-0 text-transparent opacity-0"
      }`}
    />
  );

  const edgeLedDot =
    ledAxis === "vertical" ? (
      <motion.span
        aria-hidden={!app.isActive}
        style={{
          color: app.indicatorColor,
          width: LED_EDGE_DOT_PX,
          height: LED_EDGE_DOT_PX,
          top: "50%",
          y: "-50%",
          ...(ledBeforeIcon ? { left: edgeLedInsetPx } : { right: edgeLedInsetPx }),
        }}
        className={`pointer-events-none absolute z-10 rounded-full bg-current transition-all duration-300 ease-out ${
          app.isActive
            ? `opacity-100 ${
                animationsEnabled
                  ? "animate-led-pulse"
                  : "shadow-[0_0_10px_2px_currentColor]"
              }`
            : "scale-0 opacity-0"
        }`}
      />
    ) : null;

  const iconNode = (
    <div className="relative shrink-0">
      {edgeLedDot}
      <DockOverlayAnchor
        side={overlayPreferredSide}
        gap={TOOLTIP_GAP_PX}
        className={`pointer-events-none whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 shadow-lg shadow-black/40 transition-all duration-300 ease-out ${
          isHovered && !menuOpen
            ? "scale-100 opacity-100"
            : "scale-90 opacity-0"
        }`}
      >
        {app.name}
      </DockOverlayAnchor>

      {menuOpen && (
        <DockOverlayAnchor
          innerRef={menuRef}
          side={menuSide}
          gap={TOOLTIP_GAP_PX}
          className="pointer-events-auto z-30 overflow-hidden whitespace-nowrap rounded-md bg-zinc-900/95 text-xs text-zinc-200 shadow-lg shadow-black/40"
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

          <div className="h-px bg-zinc-700/70" />

          <button
            type="button"
            disabled={separatorsFull}
            onClick={() => {
              setMenuOpen(false);
              onInsertSeparatorBefore?.(app.bundleId);
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Minus className="h-3.5 w-3.5" />
            {separatorBeforeLabel}
          </button>

          <button
            type="button"
            disabled={separatorsFull}
            onClick={() => {
              setMenuOpen(false);
              onInsertSeparatorAfter?.(app.bundleId);
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Minus className="h-3.5 w-3.5" />
            {separatorAfterLabel}
          </button>

          <div className="h-px bg-zinc-700/70" />

          <div className="flex items-center justify-between gap-3 px-3 py-1.5">
            <span className="text-zinc-300">Цвет индикатора</span>
            <input
              type="color"
              value={app.indicatorColorOverride ?? app.indicatorColorAuto}
              onPointerDown={() => {
                colorPickActiveRef.current = true;
              }}
              onBlur={() => {
                colorPickActiveRef.current = false;
              }}
              onChange={(event) => {
                colorPickActiveRef.current = false;
                onSetIndicatorColor?.(app.bundleId, event.target.value);
              }}
              className="h-7 w-7 cursor-pointer rounded border border-zinc-600 bg-transparent p-0"
              aria-label={`Цвет индикатора для ${app.name}`}
            />
          </div>
          {app.indicatorColorOverride && (
            <button
              type="button"
              onClick={() => {
                onSetIndicatorColor?.(app.bundleId, null);
              }}
              className="flex w-full items-center px-3 py-1.5 text-left text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200"
            >
              Сбросить к авто
            </button>
          )}

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
        </DockOverlayAnchor>
      )}

      {showFallback ? (
        <motion.div
          ref={(el) => registerRef?.(app.id, el)}
          style={{
            width: iconWidth,
            height: iconHeight,
            borderRadius: iconCornerRadius,
            scale: magnifySuppressed ? 1 : scale,
            y: bounceY,
            color: app.indicatorColor,
          }}
          className={`flex ${magnifyOriginClass} items-center justify-center bg-zinc-800 text-lg font-semibold`}
        >
          {app.name.slice(0, 2).toUpperCase()}
        </motion.div>
      ) : (
        <motion.img
          ref={(el) => registerRef?.(app.id, el)}
          style={{
            width: iconWidth,
            height: iconHeight,
            borderRadius: iconCornerRadius,
            scale: magnifySuppressed ? 1 : scale,
            y: bounceY,
          }}
          src={app.iconUrl ?? undefined}
          alt={app.name}
          draggable={false}
          onError={() => setBroken(true)}
          className={`${magnifyOriginClass} object-contain`}
        />
      )}
    </div>
  );

  return (
    // A `<div>`, not `<button>`: it now has to contain the "Remove from
    // Dock" menu's own real `<button>`, and nested `<button>`s are invalid
    // HTML. No functionality is lost — activation has never gone through
    // this element's own click handler anyway (see the native
    // `dock-click`/`hitTestIcon` dispatch in DockPanel.tsx), so `role`/
    // `aria-*` here are purely there to keep the same assistive-tech
    // semantics `<button>` had.
    // `gap` always present with a 0 fallback (never undefined) — same
    // stale-motion-style-key trap documented on the pill's width/height
    // in DockPanel, since `ledAxis` can flip on a live position change.
    <motion.div
      role="button"
      tabIndex={0}
      ref={buttonRef}
      aria-pressed={app.isActive}
      aria-label={`${app.name}${app.isActive ? " (running)" : ""}`}
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
      style={{ gap: ledAxis === "horizontal" ? iconLedGap : 0 }}
      className={`relative flex flex-col items-center outline-none ${
        isDragging ? "cursor-grabbing" : "cursor-grab"
      } ${isHovered || menuOpen ? "z-10" : "z-0"}`}
    >
      {iconNode}
      {ledAxis === "horizontal" ? horizontalLedBar : null}
    </motion.div>
  );
}
