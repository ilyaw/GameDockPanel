import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Reorder, motion, useMotionValue, useSpring, useTransform } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Settings } from "lucide-react";
import { DockIcon } from "./DockIcon";
import { DockSeparator } from "./DockSeparator";
import { DockContextMenuRow } from "./DockContextMenuRow";
import { DockPillContextMenu } from "./DockPillContextMenu";
import { useDockApps } from "../hooks/useDockApps";
import { useDockOrientation } from "../hooks/useDockOrientation";
import { useDockSettings } from "../hooks/useDockSettings";
import {
  MAX_SEPARATORS,
  BG_ANIMATION_CLASSES,
  getBackgroundPreset,
  backgroundPresetToDurationS,
  getBorderRingClasses,
  getPanelEffectPreset,
  getSizeMetrics,
  clampBorderWidthPx,
  PILL_CORNER_RADIUS_PX,
} from "../lib/constants";
import {
  measureMagnifyCenter,
  type MagnifyAxis,
} from "../lib/dockPlacement";
import type { DockItem, DockSettings } from "../lib/types";
import { countDockSeparators, isDockAppItem } from "../lib/types";
import { IS_WINDOWS, setDockRegionRelaxed } from "../lib/windowsDock";

/** Windows: icon activation goes through WebView2 pointer events (no global hook). */
// IS_WINDOWS imported from windowsDock — keep comment for dock-input readers.
/**
 * React only types `style` as known CSS properties — custom properties
 * (`--dock-glow-N`) are still valid inline style keys at runtime, just not
 * in that type. This narrow alias documents that gap at the one place it's
 * needed instead of reaching for a broader `any`.
 */
type StyleWithGlowVars = React.CSSProperties &
  Record<`--dock-glow-${number}`, string> &
  Record<"--dock-glow-angle", string>;

/** Same gap as `StyleWithGlowVars`, for the background gradient layer's own
 * custom properties (`--dock-bg-1..6`, `--dock-bg-duration`). Set on the
 * pill itself (not just the flow layer) so any descendant can read them by
 * inheritance — the panel-effect overlay tints itself from `--dock-bg-1`
 * the same way the flow layer does, without needing its own color config. */
type StyleWithBgVars = Record<`--dock-bg-${number}`, string> &
  Record<"--dock-bg-duration", string> &
  Record<"--dock-bg-angle", string> &
  Record<"--gradient-angle", string>;

type PillStyle = StyleWithGlowVars &
  StyleWithBgVars &
  Record<"--dock-border-width", string> &
  Record<"--dock-pill-radius", string>;

type BorderRingStyle = React.CSSProperties &
  Record<"--dock-border-width", string>;

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

/** Fallback if `onLayoutAnimationComplete` never fires after a reorder drop. */
const SETTLE_SAFETY_MS = 500;

const ICON_SIZE_SPRING = { mass: 0.35, stiffness: 420, damping: 32 };

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
  return pointInDOMRect(x, y, rect);
}

function pointInDOMRect(x: number, y: number, rect: DOMRect): boolean {
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

/** macOS Dock-style slot: insert before the first app icon whose center
 * lies past the cursor on the dock's *length* axis — X for `bottom`/`top`,
 * Y for `left`/`right` (`orientation.magnifyAxis`, which names the same
 * axis) — otherwise append at end. Comparing X unconditionally
 * broke vertical docks: every icon in a column shares one centerX, so a
 * Finder drop always resolved to index 0. */
function resolveInsertIndex(
  items: DockItem[],
  refs: Map<string, HTMLElement>,
  pillEl: HTMLElement | null,
  x: number,
  y: number,
  axis: MagnifyAxis,
): number {
  if (pillEl && !pointInRect(x, y, pillEl)) {
    return items.length;
  }

  const cursorMain = axis === "x" ? x : y;
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (!isDockAppItem(item)) continue;
    const el = refs.get(item.id);
    if (!el) continue;
    const centerMain = measureMagnifyCenter(el.getBoundingClientRect(), axis);
    if (cursorMain < centerMain) return i;
  }
  return items.length;
}

function findAppIdAtOrBefore(items: DockItem[], index: number): string | null {
  for (let i = Math.min(index, items.length - 1); i >= 0; i--) {
    const item = items[i];
    if (isDockAppItem(item)) return item.id;
  }
  return null;
}

function findAppIdAtOrAfter(items: DockItem[], index: number): string | null {
  for (let i = index; i < items.length; i++) {
    const item = items[i];
    if (isDockAppItem(item)) return item.id;
  }
  return null;
}

/** Insert-marker line geometry, axis-neutral: `main` is the line's center
 * along the dock's length axis, `crossStart`/`crossSize` its extent on the
 * thickness axis — all pill-relative. Rendered as a vertical hairline on
 * `bottom`/`top` docks and a horizontal one on `left`/`right`. */
interface InsertMarkerMetrics {
  main: number;
  crossStart: number;
  crossSize: number;
}

function getInsertMarkerMetrics(
  items: DockItem[],
  refs: Map<string, HTMLElement>,
  pillEl: HTMLElement,
  insertIndex: number,
  axis: MagnifyAxis,
): InsertMarkerMetrics | null {
  const pillRect = pillEl.getBoundingClientRect();
  const mainStart = (rect: DOMRect) => (axis === "x" ? rect.left : rect.top);
  const mainEnd = (rect: DOMRect) => (axis === "x" ? rect.right : rect.bottom);
  const crossStartOf = (rect: DOMRect) => (axis === "x" ? rect.top : rect.left);
  const crossSizeOf = (rect: DOMRect) => (axis === "x" ? rect.height : rect.width);

  let markerMain: number | null = null;
  let crossStart = 0;
  let crossSize = 0;

  if (insertIndex <= 0) {
    const firstId = findAppIdAtOrAfter(items, 0);
    const first = firstId ? refs.get(firstId) : null;
    if (first) {
      const rect = first.getBoundingClientRect();
      markerMain = mainStart(rect);
      crossStart = crossStartOf(rect);
      crossSize = crossSizeOf(rect);
    }
  } else if (insertIndex >= items.length) {
    const lastId = findAppIdAtOrBefore(items, items.length - 1);
    const last = lastId ? refs.get(lastId) : null;
    if (last) {
      const rect = last.getBoundingClientRect();
      markerMain = mainEnd(rect);
      crossStart = crossStartOf(rect);
      crossSize = crossSizeOf(rect);
    }
  } else {
    const prevId = findAppIdAtOrBefore(items, insertIndex - 1);
    const nextId = findAppIdAtOrAfter(items, insertIndex);
    const prev = prevId ? refs.get(prevId) : null;
    const next = nextId ? refs.get(nextId) : null;
    if (prev && next) {
      const prevRect = prev.getBoundingClientRect();
      const nextRect = next.getBoundingClientRect();
      markerMain = (mainEnd(prevRect) + mainStart(nextRect)) / 2;
      crossStart = Math.min(crossStartOf(prevRect), crossStartOf(nextRect));
      crossSize = Math.max(crossSizeOf(prevRect), crossSizeOf(nextRect));
    } else if (prev) {
      const prevRect = prev.getBoundingClientRect();
      markerMain = mainEnd(prevRect);
      crossStart = crossStartOf(prevRect);
      crossSize = crossSizeOf(prevRect);
    } else if (next) {
      const nextRect = next.getBoundingClientRect();
      markerMain = mainStart(nextRect);
      crossStart = crossStartOf(nextRect);
      crossSize = crossSizeOf(nextRect);
    }
  }

  if (markerMain === null || crossSize < 1) return null;
  const pillMain = axis === "x" ? pillRect.left : pillRect.top;
  const pillCross = axis === "x" ? pillRect.top : pillRect.left;
  return {
    main: markerMain - pillMain,
    crossStart: crossStart - pillCross,
    crossSize,
  };
}

/**
 * Renders nothing until the first `get_dock_settings` pull lands, then
 * mounts the real dock with the persisted values as the *initial* state.
 * Deliberately a mount gate, not a `jump()` after mount: every Motion-driven
 * style writes its correct value through React's own first commit this way.
 * The post-mount correction path (`MotionValue.jump` + Framer's frame
 * scheduler) proved unreliable on cold start — WKWebView can withhold
 * animation frames from the freshly shown, unfocused dock window for tens
 * of seconds, which left the dock rendered at the 56px/bottom defaults
 * while `dock-settings.json` said otherwise (found in the PROMPT_17 QA
 * pass with a persisted 44px size). Gating also removes the transient
 * default-orientation first mount on left/right/top docks.
 */
export function DockPanel() {
  // Apps stay in the outer, never-unmounted component so their snapshot
  // pull runs in parallel with the settings pull instead of behind it.
  const dockApps = useDockApps();
  const { settings, hydrated: settingsHydrated } = useDockSettings();
  if (!settingsHydrated || !dockApps.appsHydrated) return null;
  return <HydratedDockPanel settings={settings} dockApps={dockApps} />;
}

function HydratedDockPanel({
  settings,
  dockApps,
}: {
  settings: DockSettings;
  dockApps: ReturnType<typeof useDockApps>;
}) {
  const {
    items,
    itemsRef,
    activateApp,
    zoomApp,
    bouncingIds,
    reorderItems,
    commitReorder,
    removeApp,
    insertSeparator,
    removeSeparator,
    fileDragOver,
    fileDragInsertIndex,
    resolveInsertIndexRef,
    rejectPulseKey,
    showInFinder,
    quitApp,
    setIndicatorColor,
  } = dockApps;
  const separatorsFull = countDockSeparators(items) >= MAX_SEPARATORS;

  useEffect(() => {
    console.info(
      `[dock] hydrated: osWin=${IS_WINDOWS} apps=${items.filter(isDockAppItem).length} ` +
        `separators=${countDockSeparators(items)} position=${settings.dockPosition} ` +
        `iconSize=${settings.iconSizePx} animations=${settings.animationsEnabled} ` +
        `borderStyle=${settings.borderStyle} borderWidth=${settings.borderWidthPx} ` +
        `panelEffect=${settings.panelEffect} panelEffectOn=${settings.panelEffectEnabled} ` +
        `bgAnim=${settings.backgroundAnimationEnabled} bgPreset=${settings.backgroundPreset}`,
    );
  }, []);

  const orientation = useDockOrientation(settings.dockPosition);
  const iconSizeTarget = useMotionValue(settings.iconSizePx);
  const iconSizeAnimated = useSpring(iconSizeTarget, ICON_SIZE_SPRING);
  const iconSizeSyncedRef = useRef(false);
  const geometrySyncRafRef = useRef(0);
  const scheduleGeometrySyncRef = useRef<(() => void) | null>(null);
  const ledAlongThickness = orientation.ledAxis === "horizontal";
  /** Static layout numbers for the first paint — guarantees a non-zero pill
   * rect before Motion values land in the DOM. */
  const restMetrics = useMemo(
    () => getSizeMetrics(settings.iconSizePx, { ledAlongThickness }),
    [settings.iconSizePx, ledAlongThickness],
  );

  useEffect(() => {
    // First run is a no-op by construction (the motion values were created
    // from the same hydrated `settings.iconSizePx` this component mounted
    // with) — kept as a jump/set split so later settings pushes animate
    // through the spring while a remount stays snap-exact.
    if (!iconSizeSyncedRef.current) {
      iconSizeTarget.jump(settings.iconSizePx);
      iconSizeAnimated.jump(settings.iconSizePx);
      iconSizeSyncedRef.current = true;
      return;
    }

    iconSizeTarget.set(settings.iconSizePx);
  }, [settings.iconSizePx, iconSizeTarget, iconSizeAnimated]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void listen<number>("dock-icon-size-preview", (event) => {
      iconSizeTarget.set(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, [iconSizeTarget]);

  const pillThicknessPx = useTransform(iconSizeAnimated, (px) =>
    getSizeMetrics(px, { ledAlongThickness }).pillThicknessPx,
  );
  /**
   * Every flex `gap` fed by a MotionValue below is a `"<n>px"` *string*,
   * not a number: `gap` is missing from Framer Motion's px-append value-type
   * map (`numberValueTypes` in motion-dom), so post-mount imperative updates
   * would write a unitless number that CSS silently ignores — the gap then
   * stays frozen at whatever React wrote on mount (React itself does append
   * `px`), which desynced the DOM pill from the Rust length formula by
   * ~17px after every icon-size change until a remount.
   */
  const pillGapPx = useTransform(
    iconSizeAnimated,
    (px) => `${getSizeMetrics(px).dockGapPx}px`,
  );
  /**
   * `X`/`Y` here name the padding's *role* (along the growth axis vs along
   * the thickness axis), not a literal CSS side — which of `paddingInline`/
   * `paddingBlock` each one feeds is decided below, per `orientation`
   * (bottom/top: X→inline, Y→block; left/right: swapped), since
   * `paddingInline`/`paddingBlock` are writing-mode logical properties that
   * don't follow `flex-direction` on their own.
   */
  /**
   * Every flex `gap` / `padding*` fed by a MotionValue below is a `"<n>px"`
   * *string* — same px-append gap as `pillGapPx`: `paddingInline` is not in
   * Framer Motion's `numberValueTypes` map, so unitless updates are ignored.
   */
  const pillPaddingXPx = useTransform(
    iconSizeAnimated,
    (px) => `${getSizeMetrics(px).dockPaddingXPx}px`,
  );
  const pillPaddingYPx = useTransform(
    iconSizeAnimated,
    (px) => `${getSizeMetrics(px).dockPaddingYPx}px`,
  );
  const iconRowGapPx = useTransform(
    iconSizeAnimated,
    (px) => `${getSizeMetrics(px).dockGapPx}px`,
  );
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
  const [isReorderSettling, setIsReorderSettling] = useState(false);
  const isReorderSettlingRef = useRef(false);
  const orderAtDragStartRef = useRef<string[]>([]);
  const settleDebounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const settleSafetyRef = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const [isRejecting, setIsRejecting] = useState(false);
  const [insertMarker, setInsertMarker] = useState<InsertMarkerMetrics | null>(
    null,
  );
  const [pillMenuOpen, setPillMenuOpen] = useState(false);
  const [pillMenuAnchor, setPillMenuAnchor] = useState<{ x: number; y: number } | null>(
    null,
  );

  const mouseX = useMotionValue(Infinity);
  const mouseY = useMotionValue(Infinity);
  const lastNativeMoveAt = useRef(0);
  const dockHoveredRef = useRef(false);
  const contextMenuOpenCountRef = useRef(0);
  const contextMenuHitRectRef = useRef<DOMRect | null>(null);
  const suppressDockClickRef = useRef(false);

  const iconRefs = useRef<Map<string, HTMLElement>>(new Map());
  const pillRef = useRef<HTMLDivElement | null>(null);
  const pillMenuAnchorRef = useRef<HTMLDivElement | null>(null);
  const registerIconRef = (id: string, el: HTMLElement | null) => {
    if (el) iconRefs.current.set(id, el);
    else iconRefs.current.delete(id);
  };

  useEffect(() => {
    resolveInsertIndexRef.current = (x, y) =>
      resolveInsertIndex(
        items,
        iconRefs.current,
        pillRef.current,
        x,
        y,
        orientation.magnifyAxis,
      );
  }, [items, resolveInsertIndexRef, orientation.magnifyAxis]);

  useLayoutEffect(() => {
    const pillEl = pillRef.current;
    if (!fileDragOver || fileDragInsertIndex === null || !pillEl) {
      setInsertMarker(null);
      return;
    }

    const metrics = getInsertMarkerMetrics(
      items,
      iconRefs.current,
      pillEl,
      fileDragInsertIndex,
      orientation.magnifyAxis,
    );
    setInsertMarker(metrics);
  }, [items, fileDragOver, fileDragInsertIndex, iconSizeAnimated, orientation.magnifyAxis]);

  useEffect(() => {
    isDraggingRef.current = isDragging;
  }, [isDragging]);

  useEffect(() => {
    isReorderSettlingRef.current = isReorderSettling;
  }, [isReorderSettling]);

  useEffect(() => {
    return () => {
      clearTimeout(settleDebounceRef.current);
      clearTimeout(settleSafetyRef.current);
    };
  }, []);

  const handleContextMenuOpenChange = useCallback((open: boolean) => {
    const next = open
      ? contextMenuOpenCountRef.current + 1
      : Math.max(0, contextMenuOpenCountRef.current - 1);
    contextMenuOpenCountRef.current = next;
    // Menu closed — drop hold; keep relaxed if cursor is still on the pill.
    if (!open && next === 0) {
      void setDockRegionRelaxed(dockHoveredRef.current, { menuHold: false });
    }
  }, []);

  const handleContextMenuBoundsChange = useCallback((rect: DOMRect | null) => {
    contextMenuHitRectRef.current = rect;
  }, []);

  const closePillMenu = useCallback(() => {
    setPillMenuOpen(false);
    setPillMenuAnchor(null);
  }, []);

  const handlePillContextMenu = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if ((event.target as Element).closest("[data-dock-item]")) return;

      event.preventDefault();
      const pillEl = pillRef.current;
      if (!pillEl) return;

      const pillRect = pillEl.getBoundingClientRect();
      const anchor = {
        x: event.clientX - pillRect.left,
        y: event.clientY - pillRect.top,
      };
      // Clear SetWindowRgn + menu hold before the menu mounts (Windows clip race).
      void setDockRegionRelaxed(true, { menuHold: true }).then(() => {
        setPillMenuAnchor(anchor);
        setPillMenuOpen(true);
      });
    },
    [],
  );

  const openSettings = useCallback(() => {
    closePillMenu();
    invoke("open_settings").catch((error: unknown) => {
      console.error("Failed to open settings window:", error);
    });
  }, [closePillMenu]);

  const shouldSuppressDockClickAt = useCallback((x: number, y: number) => {
    if (suppressDockClickRef.current) {
      suppressDockClickRef.current = false;
      return true;
    }
    const menuRect = contextMenuHitRectRef.current;
    return menuRect !== null && pointInDOMRect(x, y, menuRect);
  }, []);

  const handleWindowsDockClick = useCallback(
    (clientX: number, clientY: number) => {
      if (!IS_WINDOWS) return;
      if (isDraggingRef.current || isReorderSettlingRef.current) return;
      if (shouldSuppressDockClickAt(clientX, clientY)) return;
      const id = hitTestIcon(iconRefs.current, clientX, clientY);
      if (id) activateApp(id);
    },
    [activateApp, shouldSuppressDockClickAt],
  );

  const handleWindowsDockDoubleClick = useCallback(
    (clientX: number, clientY: number) => {
      if (!IS_WINDOWS) return;
      if (isDraggingRef.current || isReorderSettlingRef.current) return;
      if (shouldSuppressDockClickAt(clientX, clientY)) return;
      const id = hitTestIcon(iconRefs.current, clientX, clientY);
      if (!id) return;
      const app = itemsRef.current.find(
        (candidate): candidate is Extract<DockItem, { type: "app" }> =>
          isDockAppItem(candidate) && candidate.id === id,
      );
      if (app?.isActive) zoomApp(app.bundleId);
    },
    [shouldSuppressDockClickAt, zoomApp, itemsRef],
  );

  const applyCursor = useCallback(
    (x: number, y: number) => {
      if (contextMenuOpenCountRef.current > 0) return;
      if (isDraggingRef.current || isReorderSettlingRef.current) return;
      mouseX.set(x);
      mouseY.set(y);
      setHoveredIconId(hitTestIcon(iconRefs.current, x, y));
    },
    [mouseX, mouseY],
  );

  const leaveDock = useCallback(() => {
    dockHoveredRef.current = false;
    mouseX.set(Infinity);
    mouseY.set(Infinity);
    setHoveredIconId(null);
    // Keep region relaxed while a context menu is open (menu lives outside
    // the pill hit-box after overlay grow).
    if (contextMenuOpenCountRef.current === 0) {
      void setDockRegionRelaxed(false);
    }
  }, [mouseX, mouseY]);

  const enterDock = useCallback(() => {
    dockHoveredRef.current = true;
    setHoverSessionId((id) => id + 1);
    // Relax before magnify/tooltip paint — faster than waiting for the
    // 50ms click-through poller hover transition.
    void setDockRegionRelaxed(true);
  }, []);

  /**
   * Local-only — fires on every neighbor the dragged icon crosses, not just
   * once at drop (see `reorderApps` in `useDockApps`). Persistence is a
   * separate, single call from `endDrag` below.
   */
  const handleReorder = useCallback(
    (newOrder: DockItem[]) => {
      reorderItems(newOrder);
    },
    [reorderItems],
  );

  const finishReorderSettle = useCallback(() => {
    if (!isReorderSettlingRef.current) return;
    clearTimeout(settleDebounceRef.current);
    clearTimeout(settleSafetyRef.current);
    isReorderSettlingRef.current = false;
    setIsReorderSettling(false);
    setReorderSettledId((id) => id + 1);

    mouseX.set(Infinity);
    mouseY.set(Infinity);
    setHoveredIconId(null);
  }, [mouseX, mouseY]);

  const scheduleFinishSettle = useCallback(() => {
    clearTimeout(settleDebounceRef.current);
    settleDebounceRef.current = setTimeout(() => {
      finishReorderSettle();
    }, SETTLE_DEBOUNCE_MS);
  }, [finishReorderSettle]);

  const beginDrag = useCallback(() => {
    orderAtDragStartRef.current = items.map((item) => item.id);
    setIsDragging(true);
    mouseX.set(Infinity);
    mouseY.set(Infinity);
    setHoveredIconId(null);
  }, [items, mouseX, mouseY]);

  /** The one discrete "drop" event — persists the final order exactly once
   * per drag gesture, mirroring the `sync_dock_geometry` pattern elsewhere. */
  const endDrag = useCallback(() => {
    setIsDragging(false);
    isDraggingRef.current = false;
    setIsReorderSettling(true);
    isReorderSettlingRef.current = true;
    void commitReorder();

    const orderChanged =
      orderAtDragStartRef.current.join() !== items.map((item) => item.id).join();
    if (!orderChanged) {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          finishReorderSettle();
        });
      });
    } else {
      clearTimeout(settleSafetyRef.current);
      settleSafetyRef.current = setTimeout(() => {
        finishReorderSettle();
      }, SETTLE_SAFETY_MS);
    }
  }, [items, commitReorder, finishReorderSettle]);

  /**
   * `onLayoutAnimationComplete` on a `Reorder.Item` fires once that item's
   * layout box has actually finished animating into its post-reorder
   * position. Debounced so N neighbor completions become one `centerX`
   * refresh instead of N spring re-targets on the dragged icon.
   */
  const handleItemLayoutAnimationComplete = useCallback(() => {
    if (!isReorderSettlingRef.current) return;
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
            mouseX.set(Infinity);
            mouseY.set(Infinity);
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
        await listen<DockCursorPayload>("dock-global-mousedown", (event) => {
          if (contextMenuOpenCountRef.current === 0) return;
          const rect = contextMenuHitRectRef.current;
          if (!rect) return;
          const { x, y } = event.payload;
          if (pointInDOMRect(x, y, rect)) {
            suppressDockClickRef.current = true;
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
        await listen<DockCursorPayload>("dock-click", (event) => {
          if (isDraggingRef.current || isReorderSettlingRef.current) return;
          const { x, y } = event.payload;
          if (shouldSuppressDockClickAt(x, y)) return;
          const id = hitTestIcon(iconRefs.current, x, y);
          if (id) activateApp(id);
        }),
      );

      unlisteners.push(
        await listen<DockCursorPayload>("dock-double-click", (event) => {
          if (isDraggingRef.current || isReorderSettlingRef.current) return;
          const { x, y } = event.payload;
          if (shouldSuppressDockClickAt(x, y)) return;
          const id = hitTestIcon(iconRefs.current, x, y);
          if (!id) return;
          const app = itemsRef.current.find(
            (candidate): candidate is Extract<DockItem, { type: "app" }> =>
              isDockAppItem(candidate) && candidate.id === id,
          );
          if (app?.isActive) zoomApp(app.bundleId);
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
  }, [activateApp, zoomApp, itemsRef, mouseX, mouseY, applyCursor, enterDock, shouldSuppressDockClickAt]);

  useEffect(() => {
    if (rejectPulseKey === 0) return;
    setIsRejecting(true);
    const timeout = setTimeout(() => setIsRejecting(false), REJECT_PULSE_MS);
    return () => clearTimeout(timeout);
  }, [rejectPulseKey]);

  useLayoutEffect(() => {
    const el = pillRef.current;
    if (!el) return;

    let alive = true;
    let measureRetries = 0;

    const measurePill = () => el.getBoundingClientRect();

    let syncRunning = false;
    let syncPending = false;

    const runGeometrySync = async () => {
      if (syncRunning) {
        syncPending = true;
        return;
      }
      syncRunning = true;

      try {
        while (alive) {
          syncPending = false;

          const rect = measurePill();
          if (rect.width < 1 || rect.height < 1) break;

          const iconSizePx = iconSizeAnimated.get();
          try {
            await invoke("resize_dock_window", {
              pillWidth: rect.width,
              pillHeight: rect.height,
              iconSizePx,
            });
          } catch (error: unknown) {
            console.error("[dock] resize_dock_window failed:", error);
          }

          if (!alive) break;

          // Window resize reflows the webview — re-measure before aligning the
          // native vibrancy mask so blur doesn't lag behind the CSS pill.
          await new Promise<void>((resolve) => {
            requestAnimationFrame(() => resolve());
          });

          if (!alive) break;

          const aligned = measurePill();
          console.info(
            `[dock] geometry sync: pill=${aligned.width.toFixed(1)}x${aligned.height.toFixed(1)} ` +
              `at=(${aligned.x.toFixed(1)},${aligned.y.toFixed(1)}) icon=${iconSizePx.toFixed(1)} win=${IS_WINDOWS}`,
          );
          try {
            await invoke("sync_vibrancy_pill", {
              x: aligned.x,
              y: aligned.y,
              width: aligned.width,
              height: aligned.height,
            });
          } catch (error: unknown) {
            console.error("[dock] sync_vibrancy_pill failed:", error);
          }

          if (!syncPending) break;
        }
      } finally {
        syncRunning = false;
      }
    };

    const syncDockGeometry = () => {
      if (!alive) return;

      const rect = measurePill();
      if ((rect.width < 1 || rect.height < 1) && measureRetries < 12) {
        measureRetries += 1;
        requestAnimationFrame(syncDockGeometry);
        return;
      }
      if (rect.width < 1 || rect.height < 1) return;

      void runGeometrySync();
    };

    const scheduleGeometrySync = () => {
      if (geometrySyncRafRef.current) return;
      geometrySyncRafRef.current = requestAnimationFrame(() => {
        geometrySyncRafRef.current = 0;
        syncDockGeometry();
      });
    };

    scheduleGeometrySyncRef.current = scheduleGeometrySync;

    scheduleGeometrySync();

    const initialRect = measurePill();

    let lastPillWidth = initialRect.width;
    let lastPillHeight = initialRect.height;
    let lastPillTop = initialRect.top;
    const observer = new ResizeObserver(() => {
      const rect = measurePill();
      const widthChanged = Math.abs(rect.width - lastPillWidth) > 0.5;
      const heightChanged = Math.abs(rect.height - lastPillHeight) > 0.5;
      const topChanged = Math.abs(rect.top - lastPillTop) > 0.5;
      if (widthChanged || heightChanged || topChanged) {
        lastPillWidth = rect.width;
        lastPillHeight = rect.height;
        lastPillTop = rect.top;
        scheduleGeometrySync();
      }
    });
    observer.observe(el);

    const onIconSizeFrame = () => {
      scheduleGeometrySync();
    };
    const unsubscribeIconSize = iconSizeAnimated.on("change", onIconSizeFrame);

    return () => {
      alive = false;
      scheduleGeometrySyncRef.current = null;
      unsubscribeIconSize();
      if (geometrySyncRafRef.current) {
        cancelAnimationFrame(geometrySyncRafRef.current);
        geometrySyncRafRef.current = 0;
      }
      observer.disconnect();
    };
  }, [items, iconSizeAnimated, orientation.position]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void listen<DockSettings>("dock-settings-changed", () => {
      scheduleGeometrySyncRef.current?.();
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void listen("dock-menu-overlay-closed", () => {
      scheduleGeometrySyncRef.current?.();
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, []);

  const borderWidthPx = clampBorderWidthPx(settings.borderWidthPx);
  const activeBgPreset = getBackgroundPreset(settings.backgroundPreset);

  /**
   * Decorative RGB frame — always the unified gradient-ring overlay when
   * animations are on. Suppressed during reject flash or file drag-over so
   * the pill's functional border can show through.
   */
  const showBorderRing =
    settings.animationsEnabled && !isRejecting && !fileDragOver;

  const bgAnimClasses = BG_ANIMATION_CLASSES[activeBgPreset.animation];

  const activePanelEffect = getPanelEffectPreset(settings.panelEffect);
  const panelEffectClasses = PANEL_EFFECT_CLASSES[activePanelEffect.id];
  /** HUD overlays stay macOS-only — on Windows they fight Mica/tint readability. */
  const showPanelEffect =
    !IS_WINDOWS &&
    settings.panelEffectEnabled &&
    !!panelEffectClasses &&
    !fileDragOver;

  useEffect(() => {
    console.info(
      `[dock] decor: showBorderRing=${showBorderRing} showPanelEffect=${showPanelEffect} ` +
        `reject=${isRejecting} dragOver=${fileDragOver} win=${IS_WINDOWS}`,
    );
  }, [showBorderRing, showPanelEffect, isRejecting, fileDragOver]);

  /**
   * Static border/shadow (from `staticGlowColor`) and CSS custom properties
   * for glow/background layers. When animations are on the decorative frame
   * is painted by the unified gradient-ring overlay — pill border stays
   * transparent so nothing doubles up underneath.
   */
  const pillStyle = useMemo<PillStyle>(() => {
    const preset = activeBgPreset;
    const mixed = preset.colors.map(
      (color) =>
        `color-mix(in srgb, ${color} ${Math.round(settings.backgroundIntensity * 100)}%, black)`,
    );
    const glowSpread = borderWidthPx * 4;
    return {
      borderColor: showBorderRing ? "transparent" : settings.staticGlowColor,
      boxShadow: showBorderRing
        ? "none"
        : `0 0 ${glowSpread}px 0 color-mix(in srgb, ${settings.staticGlowColor} 40%, transparent)`,
      "--dock-border-width": `${borderWidthPx}px`,
      "--dock-pill-radius": `${PILL_CORNER_RADIUS_PX}px`,
      borderRadius: "var(--dock-pill-radius)",
      "--dock-glow-1": settings.rgbGlowColors[0],
      "--dock-glow-2": settings.rgbGlowColors[1],
      "--dock-glow-3": settings.rgbGlowColors[2],
      "--dock-glow-4": settings.rgbGlowColors[3],
      "--dock-glow-5": settings.rgbGlowColors[4],
      "--dock-glow-6": settings.rgbGlowColors[5],
      "--dock-glow-angle": `${preset.angle}deg`,
      "--dock-bg-1": mixed[0],
      "--dock-bg-2": mixed[1],
      "--dock-bg-3": mixed[2],
      "--dock-bg-4": mixed[3],
      "--dock-bg-5": mixed[4],
      "--dock-bg-6": mixed[5],
      "--dock-bg-angle": `${preset.angle}deg`,
      "--gradient-angle": `${preset.angle}deg`,
      "--dock-bg-duration": `${backgroundPresetToDurationS(preset, settings.backgroundSpeed)}s`,
    };
  }, [
    activeBgPreset,
    borderWidthPx,
    settings.staticGlowColor,
    settings.rgbGlowColors,
    settings.backgroundIntensity,
    settings.backgroundSpeed,
    showBorderRing,
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
    <div
      className={`pointer-events-none fixed inset-0 z-50 flex overflow-visible ${orientation.wrapperClassName}`}
    >
      <motion.div
        ref={pillRef}
        data-dock-pill
        onContextMenu={handlePillContextMenu}
        onMouseEnter={enterDock}
        onMouseDown={(event) => {
          if (!IS_WINDOWS || contextMenuOpenCountRef.current === 0) return;
          const rect = contextMenuHitRectRef.current;
          if (rect && pointInDOMRect(event.clientX, event.clientY, rect)) {
            suppressDockClickRef.current = true;
          }
        }}
        onClick={(event) => {
          if (!IS_WINDOWS || event.detail > 1) return;
          handleWindowsDockClick(event.clientX, event.clientY);
        }}
        onDoubleClick={(event) => {
          if (!IS_WINDOWS) return;
          event.preventDefault();
          handleWindowsDockDoubleClick(event.clientX, event.clientY);
        }}
        onMouseMove={(event) => {
          if (contextMenuOpenCountRef.current > 0) return;
          lastNativeMoveAt.current = performance.now();
          dockHoveredRef.current = true;
          applyCursor(event.clientX, event.clientY);
        }}
        onMouseLeave={() => {
          lastNativeMoveAt.current = 0;
          leaveDock();
        }}
        style={{
          ...pillStyle,
          // Both `width` and `height` (and both `minWidth`/`minHeight`
          // below) are always present, with an explicit `"auto"` fallback
          // on the non-thickness axis rather than `undefined` — Framer
          // Motion's imperative style application can leave a stale
          // pixel value stuck on a key that silently disappears from the
          // style object between renders (observed when the orientation
          // flips right after `useDockSettings` hydrates, since the very
          // first render always starts from the `bottom` default and
          // briefly binds the *other* axis to this same motion value).
          // Always including both keys, just swapping which one holds
          // the live motion value, avoids that trap.
          width: orientation.isVertical ? pillThicknessPx : "max-content",
          height: orientation.isVertical ? "max-content" : pillThicknessPx,
          gap: pillGapPx,
          // `paddingInline`/`paddingBlock` are writing-mode logical
          // properties — they don't follow `flex-direction` on their own,
          // so which padding value (growth-axis vs thickness-axis) feeds
          // which CSS side is swapped explicitly per orientation here.
          paddingInline: orientation.isVertical ? pillPaddingYPx : pillPaddingXPx,
          paddingBlock: orientation.isVertical ? pillPaddingXPx : pillPaddingYPx,
          // Static fallback until Motion values commit on the first frame.
          minWidth: orientation.isVertical ? restMetrics.pillThicknessPx : 0,
          minHeight: orientation.isVertical ? 0 : restMetrics.pillThicknessPx,
          borderWidth: showBorderRing ? 0 : `${borderWidthPx}px`,
        }}
        className={`pointer-events-auto relative m-0 flex w-fit shrink-0 overflow-visible border bg-transparent transition-colors ${orientation.pillClassName} ${
          isRejecting ? "animate-reject-pulse" : ""
        } ${
          fileDragOver ? "border-zinc-400" : "border-transparent"
        }`}
      >
        <div
          aria-hidden
          className="dock-pill-decor-clip pointer-events-none absolute inset-0 z-0"
        >
          {settings.backgroundAnimationEnabled && !fileDragOver && (
            // Isolated clip wrapper: `filter: blur()` on an oversized layer
            // can bleed past a shared parent clip on one side in WKWebView.
            <div
              aria-hidden
              className="dock-pill-decor-clip pointer-events-none absolute inset-0"
            >
              <div
                className={`${bgAnimClasses} pointer-events-none absolute -inset-8 blur-2xl`}
                style={bgFlowStyle}
              />
            </div>
          )}
          <div
            aria-hidden
            className={`pointer-events-none absolute inset-0 ${
              fileDragOver ? "bg-zinc-900/90" : "bg-black/40"
            }`}
          />
        </div>
        {showBorderRing && (
          <div
            aria-hidden
            className="dock-border-clip pointer-events-none absolute inset-0 z-[1]"
          >
            <div
              aria-hidden
              className={`dock-border-ring pointer-events-none absolute inset-0 ${getBorderRingClasses(settings.borderStyle)}`}
              style={
                { "--dock-border-width": `${borderWidthPx}px` } as BorderRingStyle
              }
            />
          </div>
        )}
        {insertMarker && (
          <div
            aria-hidden
            className={`pointer-events-none absolute z-30 rounded-full bg-zinc-200/90 shadow-[0_0_8px_2px_rgb(255_255_255/0.35)] ${
              orientation.magnifyAxis === "x"
                ? "w-0.5 -translate-x-1/2"
                : "h-0.5 -translate-y-1/2"
            }`}
            style={
              orientation.magnifyAxis === "x"
                ? {
                    left: insertMarker.main,
                    top: insertMarker.crossStart,
                    height: insertMarker.crossSize,
                  }
                : {
                    top: insertMarker.main,
                    left: insertMarker.crossStart,
                    width: insertMarker.crossSize,
                  }
            }
          />
        )}
        <Reorder.Group
          axis={orientation.reorderAxis}
          values={items}
          onReorder={handleReorder}
          style={{ gap: iconRowGapPx }}
          className={`relative z-[2] m-0 flex list-none p-0 ${orientation.pillClassName}`}
          as="ul"
        >
        {items.map((item) => (
          <Reorder.Item
            key={item.id}
            value={item}
            layout="position"
            className="relative list-none"
            whileDrag={{ zIndex: 20 }}
            onDragStart={beginDrag}
            onDragEnd={endDrag}
            onLayoutAnimationComplete={handleItemLayoutAnimationComplete}
          >
            {isDockAppItem(item) ? (
              <DockIcon
                app={item}
                iconSizePx={iconSizeAnimated}
                magnifyNeighborStrength={settings.magnifyNeighborStrength}
                registerRef={registerIconRef}
                mouseX={mouseX}
                mouseY={mouseY}
                magnifyAxis={orientation.magnifyAxis}
                magnifyTransformOrigin={orientation.magnifyTransformOrigin}
                overlayPreferredSide={orientation.overlayPreferredSide}
                ledAxis={orientation.ledAxis}
                ledBeforeIcon={orientation.ledBeforeIcon}
                isHovered={
                  !isDragging && !isReorderSettling && hoveredIconId === item.id
                }
                hoverSessionId={hoverSessionId}
                reorderSettledId={reorderSettledId}
                isDragging={isDragging}
                isReorderSettling={isReorderSettling}
                animationsEnabled={settings.animationsEnabled}
                isBouncing={bouncingIds.has(item.id)}
                onContextMenuOpenChange={handleContextMenuOpenChange}
                onContextMenuBoundsChange={handleContextMenuBoundsChange}
                onRemove={removeApp}
                onShowInFinder={showInFinder}
                onQuit={quitApp}
                onSetIndicatorColor={setIndicatorColor}
                onInsertSeparatorBefore={(bundleId) => {
                  void insertSeparator(bundleId, "before");
                }}
                onInsertSeparatorAfter={(bundleId) => {
                  void insertSeparator(bundleId, "after");
                }}
                separatorsFull={separatorsFull}
              />
            ) : (
              <DockSeparator
                id={item.id}
                iconSizePx={iconSizeAnimated}
                isVertical={orientation.isVertical}
                overlayPreferredSide={orientation.overlayPreferredSide}
                onRemove={removeSeparator}
                isDragging={isDragging}
                onContextMenuOpenChange={handleContextMenuOpenChange}
                onContextMenuBoundsChange={handleContextMenuBoundsChange}
              />
            )}
          </Reorder.Item>
        ))}
        </Reorder.Group>
        {pillMenuOpen && pillMenuAnchor && (
          <div
            className="pointer-events-none absolute z-[30]"
            style={{ left: pillMenuAnchor.x, top: pillMenuAnchor.y }}
          >
            <DockPillContextMenu
              open={pillMenuOpen}
              anchorRef={pillMenuAnchorRef}
              overlayPreferredSide={orientation.overlayPreferredSide}
              onClose={closePillMenu}
              onContextMenuOpenChange={handleContextMenuOpenChange}
              onContextMenuBoundsChange={handleContextMenuBoundsChange}
            >
              <DockContextMenuRow onClick={openSettings}>
                <Settings className="h-3.5 w-3.5" />
                Настройки
              </DockContextMenuRow>
            </DockPillContextMenu>
            <div ref={pillMenuAnchorRef} className="h-px w-px" aria-hidden />
          </div>
        )}
        {showPanelEffect && (
          // Painted above the icon row (see `.dock-panel-scanline`/
          // `.dock-panel-hologram`'s `mix-blend-mode: screen` in
          // index.css) so it reads as ambient light sweeping across the
          // glass rather than a shadow under it — screen mode only ever
          // adds light, so it never hides an icon underneath it, even
          // where the two visually overlap.
          <div
            aria-hidden
            className="dock-panel-effect-clip pointer-events-none absolute inset-0 z-[5]"
          >
            <div
              className={`h-full w-full ${panelEffectClasses.overlay} ${panelEffectClasses.animation}`}
            />
          </div>
        )}
      </motion.div>
    </div>
  );
}
