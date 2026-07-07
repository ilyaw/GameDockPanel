import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Reorder, motion, useMotionValue, useSpring, useTransform } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Settings } from "lucide-react";
import { DockIcon } from "./DockIcon";
import { useDockApps } from "../hooks/useDockApps";
import { useDockSettings } from "../hooks/useDockSettings";
import {
  PILL_HEIGHT_PX,
  ICON_SIZE_PX,
  MAGNIFY_INFLUENCE_RADIUS_PX,
  MAGNIFY_MAX_SCALE,
  TOOLTIP_GAP_PX,
  getBackgroundPreset,
  backgroundSpeedToDurationS,
  getBorderStylePreset,
  getPanelEffectPreset,
} from "../lib/constants";
import type { DockApp } from "../lib/types";

/**
 * React only types `style` as known CSS properties — custom properties
 * (`--dock-glow-N`) are still valid inline style keys at runtime, just not
 * in that type. This narrow alias documents that gap at the one place it's
 * needed instead of reaching for a broader `any`.
 */
type StyleWithGlowVars = React.CSSProperties & Record<`--dock-glow-${number}`, string>;

/** Same gap as `StyleWithGlowVars`, for the background gradient layer's own
 * custom properties (`--dock-bg-1..6`, `--dock-bg-duration`). Set on the
 * pill itself (not just the flow layer) so any descendant can read them by
 * inheritance — the panel-effect overlay tints itself from `--dock-bg-1`
 * the same way the flow layer does, without needing its own color config. */
type StyleWithBgVars = Record<`--dock-bg-${number}`, string> &
  Record<"--dock-bg-duration", string>;

type PillStyle = StyleWithGlowVars & StyleWithBgVars;

/** Maps a `PANEL_EFFECT_PRESETS` id to its `index.css` overlay class + the
 * `--animate-panel-*` utility that drives it — kept here rather than on the
 * preset objects themselves since these are CSS implementation details the
 * settings UI never needs. */
const PANEL_EFFECT_CLASSES: Record<string, { overlay: string; animation: string }> = {
  scanline: { overlay: "dock-panel-scanline", animation: "animate-panel-scanline" },
  grid: { overlay: "dock-panel-grid", animation: "animate-panel-grid" },
  flicker: { overlay: "dock-panel-hologram", animation: "animate-panel-flicker" },
};

/** How long the pill's reject-pulse border stays applied — mirrors the
 * `--animate-reject-pulse` duration in `index.css`; kept here instead of
 * imported since it's a one-shot JS timer, not a CSS-consumed constant. */
const REJECT_PULSE_MS = 400;

/** Quiet period after the last `onLayoutAnimationComplete` before magnify
 * resumes — batches N neighbor completions into one `centerX` refresh. */
const SETTLE_DEBOUNCE_MS = 40;

const MAGNIFY_SPRING = { mass: 0.15, stiffness: 300, damping: 25 };

interface DockCursorPayload {
  x: number;
  y: number;
}

function pointInRect(
  x: number,
  y: number,
  el: HTMLElement,
): boolean {
  const rect = el.getBoundingClientRect();
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

function hitTestIcon(
  refs: Map<string, HTMLElement>,
  x: number,
  y: number,
): string | null {
  let bestId: string | null = null;
  let bestDistSq = Infinity;

  for (const [id, el] of refs) {
    const rect = el.getBoundingClientRect();
    if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) {
      continue;
    }
    const cx = rect.left + rect.width / 2;
    const cy = rect.top + rect.height / 2;
    const distSq = (x - cx) ** 2 + (y - cy) ** 2;
    if (distSq < bestDistSq) {
      bestDistSq = distSq;
      bestId = id;
    }
  }
  return bestId;
}

export function DockPanel() {
  const {
    apps,
    activateApp,
    reorderApps,
    commitReorder,
    removeApp,
    fileDragOver,
    rejectPulseKey,
    showInFinder,
    quitApp,
  } = useDockApps();
  const { settings } = useDockSettings();
  const [hoveredIconId, setHoveredIconId] = useState<string | null>(null);
  const [isSettingsHovered, setIsSettingsHovered] = useState(false);
  const [hoverSessionId, setHoverSessionId] = useState(0);
  /**
   * Bumped once per `Reorder.Item` whose layout box actually finished
   * animating into its new position after a drag-reorder. `hoverSessionId`
   * alone doesn't cover this: a reorder-drag starts and ends while the
   * cursor never leaves the pill, so no new hover session begins, and
   * `ResizeObserver` in `DockIcon` only reacts to size, not position — so
   * without this, magnify would score distance against a stale pre-reorder
   * `centerX` until some unrelated later hover-session start. Consumed by
   * the same recalculation effect as `hoverSessionId` in `DockIcon`.
   */
  const [reorderSettledId, setReorderSettledId] = useState(0);
  const [isDragging, setIsDragging] = useState(false);
  const isDraggingRef = useRef(false);
  const [isReorderSettling, setIsReorderSettling] = useState(false);
  const isReorderSettlingRef = useRef(false);
  const draggedAppIdRef = useRef<string | null>(null);
  const orderAtDragStartRef = useRef<string[]>([]);
  const layoutCompleteSeenRef = useRef(false);
  const settleDebounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const lastCursorRef = useRef({ x: Infinity, y: Infinity });
  const [isRejecting, setIsRejecting] = useState(false);

  const mouseX = useMotionValue(Infinity);
  /** Separate cursor channel for the settings slot — when the pointer is over
   * settings, `mouseX` is forced to `Infinity` so app icons don't pick up a
   * distant magnify curve from the far-right cursor X (which was making the
   * leftmost icon grow). Settings reads this value instead. */
  const settingsMouseX = useMotionValue(Infinity);
  const lastNativeMoveAt = useRef(0);
  const dockHoveredRef = useRef(false);

  const iconRefs = useRef<Map<string, HTMLElement>>(new Map());
  const pillRef = useRef<HTMLDivElement>(null);
  const settingsSlotRef = useRef<HTMLDivElement>(null);
  const settingsCenterX = useMotionValue(0);
  const registerIconRef = (id: string, el: HTMLElement | null) => {
    if (el) iconRefs.current.set(id, el);
    else iconRefs.current.delete(id);
  };

  useEffect(() => {
    isDraggingRef.current = isDragging;
  }, [isDragging]);

  useEffect(() => {
    isReorderSettlingRef.current = isReorderSettling;
  }, [isReorderSettling]);

  useEffect(() => {
    return () => clearTimeout(settleDebounceRef.current);
  }, []);

  const applyCursor = useCallback(
    (x: number, y: number) => {
      lastCursorRef.current = { x, y };
      if (isDraggingRef.current || isReorderSettlingRef.current) return;
      const settingsEl = settingsSlotRef.current;
      if (settingsEl && pointInRect(x, y, settingsEl)) {
        mouseX.set(Infinity);
        settingsMouseX.set(x);
        setHoveredIconId(null);
        setIsSettingsHovered(true);
        return;
      }
      settingsMouseX.set(Infinity);
      setIsSettingsHovered(false);
      mouseX.set(x);
      setHoveredIconId(hitTestIcon(iconRefs.current, x, y));
    },
    [mouseX, settingsMouseX],
  );

  const leaveDock = useCallback(() => {
    dockHoveredRef.current = false;
    lastCursorRef.current = { x: Infinity, y: Infinity };
    mouseX.set(Infinity);
    settingsMouseX.set(Infinity);
    setHoveredIconId(null);
    setIsSettingsHovered(false);
  }, [mouseX, settingsMouseX]);

  const enterDock = useCallback(() => {
    dockHoveredRef.current = true;
    setHoverSessionId((id) => id + 1);
  }, []);

  /**
   * Local-only — fires on every neighbor the dragged icon crosses, not just
   * once at drop (see `reorderApps` in `useDockApps`). Persistence is a
   * separate, single call from `endDrag` below.
   */
  const handleReorder = useCallback(
    (newOrder: DockApp[]) => {
      reorderApps(newOrder);
    },
    [reorderApps],
  );

  const finishReorderSettle = useCallback(() => {
    if (!isReorderSettlingRef.current) return;
    clearTimeout(settleDebounceRef.current);
    isReorderSettlingRef.current = false;
    setIsReorderSettling(false);
    draggedAppIdRef.current = null;
    layoutCompleteSeenRef.current = false;
    setReorderSettledId((id) => id + 1);

    mouseX.set(Infinity);
    settingsMouseX.set(Infinity);
    setHoveredIconId(null);
    setIsSettingsHovered(false);
  }, [mouseX, settingsMouseX]);

  const scheduleFinishSettle = useCallback(() => {
    clearTimeout(settleDebounceRef.current);
    settleDebounceRef.current = setTimeout(() => {
      finishReorderSettle();
    }, SETTLE_DEBOUNCE_MS);
  }, [finishReorderSettle]);

  const beginDrag = useCallback(
    (appId: string) => {
      draggedAppIdRef.current = appId;
      orderAtDragStartRef.current = apps.map((app) => app.id);
      setIsDragging(true);
      mouseX.set(Infinity);
      settingsMouseX.set(Infinity);
      setHoveredIconId(null);
      setIsSettingsHovered(false);
    },
    [apps, mouseX, settingsMouseX],
  );

  /** The one discrete "drop" event — persists the final order exactly once
   * per drag gesture, mirroring the `sync_dock_geometry` pattern elsewhere. */
  const endDrag = useCallback(() => {
    setIsDragging(false);
    isDraggingRef.current = false;
    setIsReorderSettling(true);
    isReorderSettlingRef.current = true;
    layoutCompleteSeenRef.current = false;
    void commitReorder();

    const orderChanged =
      orderAtDragStartRef.current.join() !== apps.map((app) => app.id).join();
    if (!orderChanged) {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          finishReorderSettle();
        });
      });
    }
  }, [apps, commitReorder, finishReorderSettle]);

  /**
   * `onLayoutAnimationComplete` on a `Reorder.Item` fires once that item's
   * layout box has actually finished animating into its post-reorder
   * position. Debounced so N neighbor completions become one `centerX`
   * refresh instead of N spring re-targets on the dragged icon.
   */
  const handleItemLayoutAnimationComplete = useCallback(() => {
    if (!isReorderSettlingRef.current) return;
    layoutCompleteSeenRef.current = true;
    scheduleFinishSettle();
  }, [scheduleFinishSettle]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void (async () => {
      unlisteners.push(
        await listen<boolean>("dock-hover", (event) => {
          if (event.payload) {
            enterDock();
          } else {
            dockHoveredRef.current = false;
            lastCursorRef.current = { x: Infinity, y: Infinity };
            mouseX.set(Infinity);
            settingsMouseX.set(Infinity);
            setHoveredIconId(null);
            setIsSettingsHovered(false);
          }
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
        return;
      }

      unlisteners.push(
        await listen<DockCursorPayload>("dock-cursor", (event) => {
          if (performance.now() - lastNativeMoveAt.current < 80) {
            return;
          }
          const { x, y } = event.payload;
          applyCursor(x, y);
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
        return;
      }

      unlisteners.push(
        await listen<DockCursorPayload>("dock-click", (event) => {
          if (isDraggingRef.current || isReorderSettlingRef.current) return;
          const { x, y } = event.payload;
          const settingsEl = settingsSlotRef.current;
          if (settingsEl && pointInRect(x, y, settingsEl)) {
            void invoke("open_settings");
            return;
          }
          const id = hitTestIcon(iconRefs.current, x, y);
          if (id) activateApp(id);
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
      }
    })();

    return () => {
      cancelled = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [activateApp, mouseX, applyCursor, enterDock]);

  useEffect(() => {
    if (rejectPulseKey === 0) return;
    setIsRejecting(true);
    const timeout = setTimeout(() => setIsRejecting(false), REJECT_PULSE_MS);
    return () => clearTimeout(timeout);
  }, [rejectPulseKey]);

  useLayoutEffect(() => {
    const el = pillRef.current;
    if (!el) return;

    let cancelled = false;

    const measurePill = () => el.getBoundingClientRect();

    const syncVibrancyFromDom = async () => {
      const rect = measurePill();
      await invoke("sync_vibrancy_pill", {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
      });
    };

    const syncDockGeometry = async () => {
      const rect = measurePill();
      await invoke("resize_dock_window", { pillWidth: rect.width });
      if (cancelled) return;
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      if (cancelled) return;
      await syncVibrancyFromDom();
    };

    void syncDockGeometry();

    let lastPillWidth = measurePill().width;
    const observer = new ResizeObserver(() => {
      const width = measurePill().width;
      if (Math.abs(width - lastPillWidth) > 0.5) {
        lastPillWidth = width;
        void syncDockGeometry();
      } else {
        void syncVibrancyFromDom();
      }
    });
    observer.observe(el);
    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, [apps]);

  useLayoutEffect(() => {
    const el = settingsSlotRef.current;
    if (!el) return;

    const measure = () => {
      const rect = el.getBoundingClientRect();
      settingsCenterX.set(rect.left + rect.width / 2);
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [settingsCenterX, apps, hoverSessionId, reorderSettledId]);

  const settingsDistance = useTransform(
    [settingsMouseX, settingsCenterX],
    ([mx, cx]: number[]) => {
      if (!Number.isFinite(mx)) return Infinity;
      return mx - cx;
    },
  );
  const settingsScaleRaw = useTransform(
    settingsDistance,
    [-MAGNIFY_INFLUENCE_RADIUS_PX, 0, MAGNIFY_INFLUENCE_RADIUS_PX],
    [1, MAGNIFY_MAX_SCALE, 1],
  );
  const settingsScale = useSpring(settingsScaleRaw, MAGNIFY_SPRING);

  const activeBorderStyle = getBorderStylePreset(settings.borderStyle);
  /**
   * "Scan" doesn't animate `border-color`/`box-shadow` like the other three
   * styles — it gets a dedicated conic-gradient ring overlay instead (see
   * `.dock-border-scan-ring` in index.css). Suppressed during a reject
   * flash or an active file drag-over, same as the other border styles,
   * since both of those already communicate their own state through the
   * pill's real border.
   */
  const showScanRing =
    settings.animationsEnabled &&
    activeBorderStyle.id === "scan" &&
    !isRejecting &&
    !fileDragOver;

  const activePanelEffect = getPanelEffectPreset(settings.panelEffect);
  const panelEffectClasses = PANEL_EFFECT_CLASSES[activePanelEffect.id];
  const showPanelEffect =
    settings.panelEffectEnabled && !!panelEffectClasses && !fileDragOver;

  /**
   * Static border/shadow (from `staticGlowColor`) sit alongside the CSS
   * custom properties the animated cycle reads (`--dock-glow-1..6`) and
   * the background flow's own (`--dock-bg-1..6`/`--dock-bg-duration`) —
   * the latter live here (not just on the flow layer below) so the
   * panel-effect overlay can read them too, by inheritance, without a
   * second copy. Border custom properties live in the same inline style
   * rather than switching classes: a running CSS animation
   * (`animate-rgb-glow`/`animate-border-pulse`/etc., or
   * `animate-reject-pulse`) always outranks a plain inline declaration on
   * the same property per the cascade, so these static values simply show
   * through whenever no animation class is applied (`animationsEnabled`
   * off, not rejecting) — no extra branching needed to pick which one
   * "wins". The scan ring is the one exception: it fully replaces the
   * visible border, so `borderColor`/`boxShadow` are forced transparent
   * while it's showing instead of leaving a static color to peek through
   * underneath it.
   */
  const pillStyle = useMemo<PillStyle>(() => {
    const preset = getBackgroundPreset(settings.backgroundPreset);
    const mixed = preset.colors.map(
      (color) =>
        `color-mix(in srgb, ${color} ${Math.round(settings.backgroundIntensity * 100)}%, black)`,
    );
    return {
      height: PILL_HEIGHT_PX,
      borderColor: showScanRing ? "transparent" : settings.staticGlowColor,
      boxShadow: showScanRing
        ? "none"
        : `0 0 14px 0 color-mix(in srgb, ${settings.staticGlowColor} 40%, transparent)`,
      "--dock-glow-1": settings.rgbGlowColors[0],
      "--dock-glow-2": settings.rgbGlowColors[1],
      "--dock-glow-3": settings.rgbGlowColors[2],
      "--dock-glow-4": settings.rgbGlowColors[3],
      "--dock-glow-5": settings.rgbGlowColors[4],
      "--dock-glow-6": settings.rgbGlowColors[5],
      "--dock-bg-1": mixed[0],
      "--dock-bg-2": mixed[1],
      "--dock-bg-3": mixed[2],
      "--dock-bg-4": mixed[3],
      "--dock-bg-5": mixed[4],
      "--dock-bg-6": mixed[5],
      "--dock-bg-duration": `${backgroundSpeedToDurationS(settings.backgroundSpeed)}s`,
    };
  }, [
    settings.staticGlowColor,
    settings.rgbGlowColors,
    settings.backgroundPreset,
    settings.backgroundIntensity,
    settings.backgroundSpeed,
    showScanRing,
  ]);

  /** Just the flow layer's own opacity now — its color/duration custom
   * properties moved onto `pillStyle` above so other overlays can share
   * them. `backgroundVisibility` stays this layer's own `opacity` (not
   * baked into each color stop) so the gradient's relative color balance
   * stays constant as the slider moves. */
  const bgFlowStyle = useMemo<React.CSSProperties>(
    () => ({ opacity: settings.backgroundVisibility }),
    [settings.backgroundVisibility],
  );

  return (
    <div className="pointer-events-none fixed inset-0 z-50 flex flex-col justify-end overflow-visible pb-2">
      <div
        ref={pillRef}
        onMouseEnter={enterDock}
        onMouseMove={(event) => {
          lastNativeMoveAt.current = performance.now();
          dockHoveredRef.current = true;
          applyCursor(event.clientX, event.clientY);
        }}
        onMouseLeave={() => {
          lastNativeMoveAt.current = 0;
          leaveDock();
        }}
        style={pillStyle}
        className={`pointer-events-auto relative mx-auto m-0 flex shrink-0 items-end gap-2 overflow-visible rounded-[28px] border px-5 py-3 transition-colors ${
          isRejecting
            ? "animate-reject-pulse"
            : settings.animationsEnabled && !showScanRing
              ? activeBorderStyle.animationClass
              : ""
        } ${
          fileDragOver
            ? "border-zinc-400 bg-zinc-900/90"
            : "border-transparent bg-black/40"
        }`}
      >
        {showScanRing && (
          // Separate overlay, not a class on the pill itself — see the
          // "scan" branch note on `showScanRing`/`activeBorderStyle` above
          // for why a rotating gradient can't just replace `border-color`
          // like the other three styles do.
          <div
            aria-hidden
            className="dock-border-scan-ring animate-border-scan-rotate pointer-events-none absolute -inset-px -z-10 rounded-[28px]"
          />
        )}
        {settings.backgroundAnimationEnabled && !fileDragOver && (
          // Negative z-index puts this behind the icons/button below
          // automatically (static in-flow siblings paint above negative-
          // z-index descendants per the stacking spec) — no z-index needed
          // on them. The outer wrapper stays unblurred so it can clip to
          // the pill's own rounded corners; the inner layer is the one
          // that's oversized + blurred, so the blur's soft falloff never
          // shows a hard, unblurred edge at the clip boundary.
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0 -z-10 overflow-hidden rounded-[28px]"
          >
            <div
              className="dock-bg-flow animate-rgb-bg-flow absolute -inset-8 blur-2xl"
              style={bgFlowStyle}
            />
          </div>
        )}
        <Reorder.Group
          axis="x"
          values={apps}
          onReorder={handleReorder}
          className="m-0 flex list-none items-end gap-2"
          as="ul"
        >
        {apps.map((app) => (
          // Dragging an icon out of the pill and releasing just snaps it
          // back into its (possibly reordered) slot — a deliberate decision,
          // not an unexamined default: Framer Motion hardcodes
          // `dragSnapToOrigin: true` inside `Reorder.Item` (spread after
          // `...props`, so it can't be overridden via a prop anyway), and a
          // "drag out to remove" gesture was explicitly considered and
          // rejected as out of scope for this pass.
          <Reorder.Item
            key={app.id}
            value={app}
            // `layout="position"` (not the default `true`) opts out of
            // framer-motion's layout *size* tracking. Icons never change
            // size on reorder — only position — so the size half is dead
            // weight; worse, framer-motion's automatic scale-correction for
            // size changes stacks with `DockIcon`'s own independent magnify
            // `scale` spring on the same element/descendants, producing a
            // visible jitter on exactly the just-dragged icon (the only one
            // still carrying a live size-projection box) and throwing off
            // the context menu's real hit-box relative to where it's drawn.
            layout="position"
            className="relative list-none"
            whileDrag={{ zIndex: 20 }}
            onDragStart={() => beginDrag(app.id)}
            onDragEnd={endDrag}
            onLayoutAnimationComplete={handleItemLayoutAnimationComplete}
          >
            <DockIcon
              app={app}
              registerRef={registerIconRef}
              mouseX={mouseX}
              isHovered={
                !isDragging && !isReorderSettling && hoveredIconId === app.id
              }
              hoverSessionId={hoverSessionId}
              reorderSettledId={reorderSettledId}
              isDragging={isDragging}
              isReorderSettling={isReorderSettling}
              animationsEnabled={settings.animationsEnabled}
              onRemove={removeApp}
              onShowInFinder={showInFinder}
              onQuit={quitApp}
            />
          </Reorder.Item>
        ))}
        </Reorder.Group>
        <div
          aria-hidden
          className="mx-1 mb-3 w-px shrink-0 self-end bg-zinc-600/80"
          style={{ height: ICON_SIZE_PX * 0.55 }}
        />
        {/* Matches DockIcon's own `flex-col items-center gap-2` shape (icon +
            gap + LED) with an invisible spacer standing in for the LED — the
            pill row is `items-end`, so without this the button's bottom
            (and thus its glyph) lands 11px lower than every app icon's. */}
        <div className="flex shrink-0 flex-col items-center gap-2">
          <div
            ref={settingsSlotRef}
            className={`relative h-14 w-14 shrink-0 ${
              isSettingsHovered && !isDragging && !isReorderSettling ? "z-10" : ""
            }`}
          >
            <span
              style={{ marginBottom: TOOLTIP_GAP_PX }}
              className={`pointer-events-none absolute bottom-full left-1/2 z-20 -translate-x-1/2 whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 shadow-lg shadow-black/40 transition-all duration-300 ease-out ${
                isSettingsHovered && !isDragging && !isReorderSettling
                  ? "scale-100 opacity-100"
                  : "scale-90 opacity-0"
              }`}
            >
              Настройки
            </span>
            <motion.div
              style={{ scale: isDragging || isReorderSettling ? 1 : settingsScale }}
              className="h-full w-full origin-bottom"
            >
              <button
                type="button"
                aria-label="Настройки"
                onClick={() => {
                  void invoke("open_settings");
                }}
                style={
                  {
                    "--settings-accent": settings.staticGlowColor,
                    borderColor: `color-mix(in srgb, ${settings.staticGlowColor} 34%, transparent)`,
                    boxShadow: `inset 0 1px 0 0 rgb(255 255 255 / 0.08), 0 0 12px -2px color-mix(in srgb, ${settings.staticGlowColor} 16%, transparent)`,
                  } as React.CSSProperties & Record<"--settings-accent", string>
                }
                className="flex h-full w-full items-center justify-center rounded-2xl border bg-zinc-950/40 text-zinc-400 transition-[color,background-color,box-shadow,border-color] duration-200 hover:border-[color-mix(in_srgb,var(--settings-accent)_48%,transparent)] hover:bg-zinc-900/55 hover:text-zinc-100 hover:shadow-[inset_0_1px_0_0_rgb(255_255_255/0.12),0_0_16px_0_color-mix(in_srgb,var(--settings-accent)_26%,transparent)]"
              >
                <Settings className="h-7 w-7" strokeWidth={1.75} />
              </button>
            </motion.div>
          </div>
          <span aria-hidden className="h-[3px] w-6" />
        </div>
        {showPanelEffect && (
          // Painted above the icon row (see `.dock-panel-scanline`/
          // `.dock-panel-hologram`'s `mix-blend-mode: screen` in
          // index.css) so it reads as ambient light sweeping across the
          // glass rather than a shadow under it — screen mode only ever
          // adds light, so it never hides an icon underneath it, even
          // where the two visually overlap.
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0 z-[5] overflow-hidden rounded-[28px]"
          >
            <div
              className={`h-full w-full ${panelEffectClasses.overlay} ${panelEffectClasses.animation}`}
            />
          </div>
        )}
      </div>
    </div>
  );
}
