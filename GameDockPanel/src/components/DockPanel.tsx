import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Reorder, useMotionValue } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Settings } from "lucide-react";
import { DockIcon } from "./DockIcon";
import { useDockApps } from "../hooks/useDockApps";
import { useDockSettings } from "../hooks/useDockSettings";
import {
  PILL_HEIGHT_PX,
  ICON_SIZE_PX,
  getBackgroundPreset,
  backgroundSpeedToDurationS,
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
 * custom properties (`--dock-bg-1..6`, `--dock-bg-duration`). */
type StyleWithBgVars = React.CSSProperties &
  Record<`--dock-bg-${number}`, string> &
  Record<"--dock-bg-duration", string>;

/** How long the pill's reject-pulse border stays applied — mirrors the
 * `--animate-reject-pulse` duration in `index.css`; kept here instead of
 * imported since it's a one-shot JS timer, not a CSS-consumed constant. */
const REJECT_PULSE_MS = 400;

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
  const [isRejecting, setIsRejecting] = useState(false);

  const mouseX = useMotionValue(Infinity);
  const lastNativeMoveAt = useRef(0);
  const dockHoveredRef = useRef(false);

  const iconRefs = useRef<Map<string, HTMLElement>>(new Map());
  const pillRef = useRef<HTMLDivElement>(null);
  const settingsButtonRef = useRef<HTMLButtonElement>(null);
  const registerIconRef = (id: string, el: HTMLElement | null) => {
    if (el) iconRefs.current.set(id, el);
    else iconRefs.current.delete(id);
  };

  useEffect(() => {
    isDraggingRef.current = isDragging;
  }, [isDragging]);

  const applyCursor = useCallback(
    (x: number, y: number) => {
      if (isDraggingRef.current) return;
      mouseX.set(x);
      setHoveredIconId(hitTestIcon(iconRefs.current, x, y));
    },
    [mouseX],
  );

  const leaveDock = useCallback(() => {
    dockHoveredRef.current = false;
    mouseX.set(Infinity);
    setHoveredIconId(null);
  }, [mouseX]);

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

  const beginDrag = useCallback(() => {
    setIsDragging(true);
    mouseX.set(Infinity);
    setHoveredIconId(null);
  }, [mouseX]);

  /** The one discrete "drop" event — persists the final order exactly once
   * per drag gesture, mirroring the `sync_dock_geometry` pattern elsewhere. */
  const endDrag = useCallback(() => {
    setIsDragging(false);
    void commitReorder();
  }, [commitReorder]);

  /**
   * `onLayoutAnimationComplete` on a `Reorder.Item` fires once that item's
   * layout box has actually finished animating into its post-reorder
   * position — after the DOM has visually settled, not mid-flight. Bumping
   * on every completion (not just the first) is deliberate: it's cheap
   * (a `getBoundingClientRect()` per icon in `DockIcon`, not a per-frame
   * cost), and the last completion to fire always leaves `centerX` correct.
   */
  const handleItemLayoutAnimationComplete = useCallback(() => {
    setReorderSettledId((id) => id + 1);
  }, []);

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
            mouseX.set(Infinity);
            setHoveredIconId(null);
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
          if (isDraggingRef.current) return;
          const { x, y } = event.payload;
          const settingsEl = settingsButtonRef.current;
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

    const syncVibrancyPill = () => {
      const rect = el.getBoundingClientRect();
      void invoke("sync_vibrancy_pill", {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
      });
    };

    syncVibrancyPill();
    const observer = new ResizeObserver(syncVibrancyPill);
    observer.observe(el);
    return () => observer.disconnect();
  }, [apps]);

  /**
   * Static border/shadow (from `staticGlowColor`) sit alongside the CSS
   * custom properties the animated cycle reads (`--dock-glow-1..6`).
   * Both live in the same inline style rather than switching classes: a
   * running CSS animation (`animate-rgb-glow` / `animate-reject-pulse`)
   * always outranks a plain inline declaration on the same property per
   * the cascade, so these static values simply show through whenever
   * neither animation class is applied (`animationsEnabled` off, not
   * rejecting) — no extra branching needed to pick which one "wins".
   */
  const pillStyle = useMemo<StyleWithGlowVars>(
    () => ({
      height: PILL_HEIGHT_PX,
      borderColor: settings.staticGlowColor,
      boxShadow: `0 0 14px 0 color-mix(in srgb, ${settings.staticGlowColor} 40%, transparent)`,
      "--dock-glow-1": settings.rgbGlowColors[0],
      "--dock-glow-2": settings.rgbGlowColors[1],
      "--dock-glow-3": settings.rgbGlowColors[2],
      "--dock-glow-4": settings.rgbGlowColors[3],
      "--dock-glow-5": settings.rgbGlowColors[4],
      "--dock-glow-6": settings.rgbGlowColors[5],
    }),
    [settings.staticGlowColor, settings.rgbGlowColors],
  );

  /**
   * The animated RGB/gradient background layer (see `.dock-bg-flow` /
   * `@keyframes rgb-bg-flow` in index.css) — `backgroundIntensity` mixes
   * each preset color toward black (same `color-mix()` technique as the
   * border glow's shadow above), `backgroundVisibility` becomes this
   * layer's own `opacity` rather than baking alpha into each color stop
   * so the gradient's relative color balance stays constant as the
   * slider moves, and `backgroundSpeed` maps to a duration consumed by
   * `--animate-rgb-bg-flow`'s `var(--dock-bg-duration, ...)` fallback.
   */
  const bgFlowStyle = useMemo<StyleWithBgVars>(() => {
    const preset = getBackgroundPreset(settings.backgroundPreset);
    const mixed = preset.colors.map(
      (color) =>
        `color-mix(in srgb, ${color} ${Math.round(settings.backgroundIntensity * 100)}%, black)`,
    );
    return {
      opacity: settings.backgroundVisibility,
      "--dock-bg-1": mixed[0],
      "--dock-bg-2": mixed[1],
      "--dock-bg-3": mixed[2],
      "--dock-bg-4": mixed[3],
      "--dock-bg-5": mixed[4],
      "--dock-bg-6": mixed[5],
      "--dock-bg-duration": `${backgroundSpeedToDurationS(settings.backgroundSpeed)}s`,
    };
  }, [
    settings.backgroundPreset,
    settings.backgroundIntensity,
    settings.backgroundVisibility,
    settings.backgroundSpeed,
  ]);

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
            : settings.animationsEnabled
              ? "animate-rgb-glow"
              : ""
        } ${
          fileDragOver
            ? "border-zinc-400 bg-zinc-900/90"
            : "border-transparent bg-black/40"
        }`}
      >
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
            whileDrag={{ zIndex: 20, scale: 1.08 }}
            onDragStart={beginDrag}
            onDragEnd={endDrag}
            onLayoutAnimationComplete={handleItemLayoutAnimationComplete}
          >
            <DockIcon
              app={app}
              registerRef={registerIconRef}
              mouseX={mouseX}
              isHovered={!isDragging && hoveredIconId === app.id}
              hoverSessionId={hoverSessionId}
              reorderSettledId={reorderSettledId}
              isDragging={isDragging}
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
          <button
            ref={settingsButtonRef}
            type="button"
            title="Settings"
            aria-label="Settings"
            onClick={() => {
              void invoke("open_settings");
            }}
            className="flex h-14 w-14 shrink-0 items-center justify-center rounded-2xl text-zinc-400 transition-colors hover:bg-zinc-800/60 hover:text-zinc-100"
          >
            <Settings className="h-7 w-7" strokeWidth={1.75} />
          </button>
          <span aria-hidden className="h-[3px] w-6" />
        </div>
      </div>
    </div>
  );
}
