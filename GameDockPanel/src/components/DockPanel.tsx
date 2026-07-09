import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Reorder, motion, useMotionValue, useSpring, useTransform } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Settings } from "lucide-react";
import { DockIcon } from "./DockIcon";
import { DockRowDivider } from "./DockRowDivider";
import { DockSeparator } from "./DockSeparator";
import { DockOverlayAnchor } from "./DockOverlayAnchor";
import { useDockApps } from "../hooks/useDockApps";
import { useDockOrientation } from "../hooks/useDockOrientation";
import { useDockSettings } from "../hooks/useDockSettings";
import {
  MAX_SEPARATORS,
  TOOLTIP_GAP_PX,
  BG_ANIMATION_CLASSES,
  computeMagnifyScale,
  getBackgroundPreset,
  backgroundPresetToDurationS,
  getBorderRingClasses,
  getPanelEffectPreset,
  getSizeMetrics,
  clampBorderWidthPx,
  PILL_CORNER_RADIUS_PX,
  roundedRingMaskStyle,
} from "../lib/constants";
import {
  magnifyOriginClassName,
  measureMagnifyCenter,
  type MagnifyAxis,
} from "../lib/dockPlacement";
import type { DockItem, DockSettings } from "../lib/types";
import { countDockSeparators, isDockAppItem } from "../lib/types";

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

const MAGNIFY_SPRING = { mass: 0.15, stiffness: 300, damping: 25 };
/** Critically damped, ~100 ms settle — interpolates between rapid slider
 * steps without the old soft spring's ~1.8 s lag or jump()'s 1 px stutter. */
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
 * axis) — otherwise append before settings. Comparing X unconditionally
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
  const { settings, hydrated } = useDockSettings();
  if (!hydrated) return null;
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
  const pillPaddingXPx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).dockPaddingXPx,
  );
  const pillPaddingYPx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).dockPaddingYPx,
  );
  const iconRowGapPx = useTransform(
    iconSizeAnimated,
    (px) => `${getSizeMetrics(px).dockGapPx}px`,
  );
  /** Icon↔LED gap for the settings gear column — same scaled metric the
   * pill-thickness formula uses (`iconLedGapPx`), not a fixed `gap-2`:
   * a fixed 8px drifts against the formula at non-default icon sizes.
   * String with units — see `pillGapPx`. */
  const settingsLedGapPx = useTransform(
    iconSizeAnimated,
    (px) => `${getSizeMetrics(px).iconLedGapPx}px`,
  );
  const settingsSlotSizePx = useTransform(iconSizeAnimated, (px) => px);
  const settingsCornerRadiusPx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).iconCornerRadiusPx,
  );
  const settingsMagnifyRadiusPx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).magnifyInfluenceRadiusPx,
  );
  const magnifyNeighborStrengthMV = useMotionValue(settings.magnifyNeighborStrength);
  useEffect(() => {
    magnifyNeighborStrengthMV.set(settings.magnifyNeighborStrength);
  }, [settings.magnifyNeighborStrength, magnifyNeighborStrengthMV]);
  const settingsIconSizePx = useTransform(iconSizeAnimated, (px) => px / 2);
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
  const orderAtDragStartRef = useRef<string[]>([]);
  const settleDebounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const settleSafetyRef = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const [isRejecting, setIsRejecting] = useState(false);
  const [pillMaskSize, setPillMaskSize] = useState({ width: 0, height: 0 });
  const [insertMarker, setInsertMarker] = useState<InsertMarkerMetrics | null>(
    null,
  );

  const mouseX = useMotionValue(Infinity);
  const mouseY = useMotionValue(Infinity);
  /** Separate cursor channel for the settings slot — when the pointer is over
   * settings, the app-icon magnify axis is forced to `Infinity` so icons
   * don't pick up a distant magnify curve from the far edge cursor position. */
  const settingsMouseX = useMotionValue(Infinity);
  const settingsMouseY = useMotionValue(Infinity);
  const lastNativeMoveAt = useRef(0);
  const dockHoveredRef = useRef(false);
  const contextMenuOpenCountRef = useRef(0);
  const [contextMenuActive, setContextMenuActive] = useState(false);

  const iconRefs = useRef<Map<string, HTMLElement>>(new Map());
  const pillRef = useRef<HTMLDivElement | null>(null);
  const settingsSlotRef = useRef<HTMLDivElement>(null);
  const settingsCenterMain = useMotionValue(0);
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
    setContextMenuActive(next > 0);
  }, []);

  const applyCursor = useCallback(
    (x: number, y: number) => {
      if (contextMenuOpenCountRef.current > 0) return;
      if (isDraggingRef.current || isReorderSettlingRef.current) return;
      const settingsEl = settingsSlotRef.current;
      if (settingsEl && pointInRect(x, y, settingsEl)) {
        if (orientation.magnifyAxis === "x") {
          mouseX.set(Infinity);
          settingsMouseX.set(x);
        } else {
          mouseY.set(Infinity);
          settingsMouseY.set(y);
        }
        setHoveredIconId(null);
        setIsSettingsHovered(true);
        return;
      }
      settingsMouseX.set(Infinity);
      settingsMouseY.set(Infinity);
      setIsSettingsHovered(false);
      mouseX.set(x);
      mouseY.set(y);
      setHoveredIconId(hitTestIcon(iconRefs.current, x, y));
    },
    [
      mouseX,
      mouseY,
      settingsMouseX,
      settingsMouseY,
      orientation.magnifyAxis,
    ],
  );

  const leaveDock = useCallback(() => {
    dockHoveredRef.current = false;
    mouseX.set(Infinity);
    mouseY.set(Infinity);
    settingsMouseX.set(Infinity);
    settingsMouseY.set(Infinity);
    setHoveredIconId(null);
    setIsSettingsHovered(false);
  }, [mouseX, mouseY, settingsMouseX, settingsMouseY]);

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
    settingsMouseX.set(Infinity);
    settingsMouseY.set(Infinity);
    setHoveredIconId(null);
    setIsSettingsHovered(false);
  }, [mouseX, mouseY, settingsMouseX, settingsMouseY]);

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
    settingsMouseX.set(Infinity);
    settingsMouseY.set(Infinity);
    setHoveredIconId(null);
    setIsSettingsHovered(false);
  }, [items, mouseX, mouseY, settingsMouseX, settingsMouseY]);

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
            settingsMouseX.set(Infinity);
            settingsMouseY.set(Infinity);
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

      unlisteners.push(
        await listen<DockCursorPayload>("dock-double-click", (event) => {
          if (isDraggingRef.current || isReorderSettlingRef.current) return;
          const { x, y } = event.payload;
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
  }, [activateApp, zoomApp, itemsRef, mouseX, mouseY, applyCursor, enterDock]);

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
          await invoke("resize_dock_window", {
            pillWidth: rect.width,
            pillHeight: rect.height,
            iconSizePx,
          });

          if (!alive) break;

          // Window resize reflows the webview — re-measure before aligning the
          // native vibrancy mask so blur doesn't lag behind the CSS pill.
          await new Promise<void>((resolve) => {
            requestAnimationFrame(() => resolve());
          });

          if (!alive) break;

          const aligned = measurePill();
          await invoke("sync_vibrancy_pill", {
            x: aligned.x,
            y: aligned.y,
            width: aligned.width,
            height: aligned.height,
          });

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
    setPillMaskSize({
      width: initialRect.width,
      height: initialRect.height,
    });

    let lastPillWidth = initialRect.width;
    let lastPillHeight = initialRect.height;
    let lastPillTop = initialRect.top;
    const observer = new ResizeObserver(() => {
      const rect = measurePill();
      const widthChanged = Math.abs(rect.width - lastPillWidth) > 0.5;
      const heightChanged = Math.abs(rect.height - lastPillHeight) > 0.5;
      const topChanged = Math.abs(rect.top - lastPillTop) > 0.5;
      setPillMaskSize((prev) =>
        Math.abs(prev.width - rect.width) < 0.01 &&
        Math.abs(prev.height - rect.height) < 0.01
          ? prev
          : { width: rect.width, height: rect.height },
      );
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
  }, [items, iconSizeAnimated]);

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

  useLayoutEffect(() => {
    const el = settingsSlotRef.current;
    if (!el) return;

    const measure = () => {
      const rect = el.getBoundingClientRect();
      settingsCenterMain.set(
        measureMagnifyCenter(rect, orientation.magnifyAxis),
      );
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [settingsCenterMain, items, hoverSessionId, reorderSettledId, orientation.magnifyAxis]);

  const settingsScaleRaw = useTransform(
    [
      settingsMouseX,
      settingsMouseY,
      settingsCenterMain,
      settingsMagnifyRadiusPx,
      magnifyNeighborStrengthMV,
    ],
    ([mx, my, cm, radius, neighborStrength]: number[]) => {
      const m = orientation.magnifyAxis === "x" ? mx : my;
      if (!Number.isFinite(m)) return 1;
      const distance = m - cm;
      return computeMagnifyScale(Math.abs(distance), radius, neighborStrength);
    },
  );
  const settingsScale = useSpring(settingsScaleRaw, MAGNIFY_SPRING);
  const settingsOriginClass = magnifyOriginClassName(
    orientation.magnifyTransformOrigin,
  );

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
  const showPanelEffect =
    settings.panelEffectEnabled && !!panelEffectClasses && !fileDragOver;

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

  const borderRingStyle = useMemo<BorderRingStyle>(() => {
    if (pillMaskSize.width < 1 || pillMaskSize.height < 1) {
      return { "--dock-border-width": `${borderWidthPx}px` };
    }
    return {
      "--dock-border-width": `${borderWidthPx}px`,
      ...roundedRingMaskStyle(
        pillMaskSize.width,
        pillMaskSize.height,
        PILL_CORNER_RADIUS_PX,
        borderWidthPx,
      ),
    };
  }, [borderWidthPx, pillMaskSize.height, pillMaskSize.width]);

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
        onMouseEnter={enterDock}
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
          width: orientation.isVertical ? pillThicknessPx : "auto",
          height: orientation.isVertical ? "auto" : pillThicknessPx,
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
        className={`pointer-events-auto relative m-0 flex shrink-0 overflow-visible rounded-[28px] border bg-transparent transition-colors ${orientation.pillClassName} ${
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
            // Oversized + blurred inner layer; the decor clip boundary
            // keeps the soft falloff from showing a hard edge at the pill.
            <div
              className={`${bgAnimClasses} pointer-events-none absolute -inset-8 blur-2xl`}
              style={bgFlowStyle}
            />
          )}
          <div
            aria-hidden
            className={`pointer-events-none absolute inset-0 ${
              fileDragOver ? "bg-zinc-900/90" : "bg-black/40"
            }`}
          />
          {showBorderRing && pillMaskSize.width > 0 && pillMaskSize.height > 0 && (
            <div
              aria-hidden
              className="dock-border-clip pointer-events-none absolute inset-0 z-[1]"
            >
              <div
                aria-hidden
                className={`pointer-events-none absolute inset-0 ${getBorderRingClasses(settings.borderStyle)}`}
                style={borderRingStyle}
              />
            </div>
          )}
        </div>
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
          className={`relative z-[2] m-0 flex list-none ${orientation.pillClassName}`}
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
              />
            )}
          </Reorder.Item>
        ))}
        </Reorder.Group>
        <DockRowDivider
          iconSizePx={iconSizeAnimated}
          isVertical={orientation.isVertical}
          className="relative z-[2] mx-1"
        />
        {/* Settings gear — horizontal docks reserve space for the LED bar.
            `gap` always present with a 0 fallback (never undefined) — same
            stale-motion-style-key trap as the pill's width/height above. */}
        <motion.div
          style={{
            gap: orientation.ledAxis === "horizontal" ? settingsLedGapPx : 0,
          }}
          className={`relative z-[2] flex shrink-0 ${
            orientation.ledAxis === "vertical"
              ? "items-center"
              : "flex-col items-center"
          }`}
        >
          <motion.div
            ref={settingsSlotRef}
            style={{ height: settingsSlotSizePx, width: settingsSlotSizePx }}
            className={`relative shrink-0 ${
              isSettingsHovered &&
              !isDragging &&
              !isReorderSettling &&
              !contextMenuActive
                ? "z-10"
                : ""
            }`}
          >
            <DockOverlayAnchor
              side={orientation.overlayPreferredSide}
              gap={TOOLTIP_GAP_PX}
              className={`pointer-events-none whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 shadow-lg shadow-black/40 transition-all duration-300 ease-out ${
                isSettingsHovered &&
                !isDragging &&
                !isReorderSettling &&
                !contextMenuActive
                  ? "scale-100 opacity-100"
                  : "scale-90 opacity-0"
              }`}
            >
              Настройки
            </DockOverlayAnchor>
            <motion.div
              style={{
                scale: isDragging || isReorderSettling ? 1 : settingsScale,
              }}
              className={`h-full w-full ${settingsOriginClass}`}
            >
              <motion.button
                type="button"
                aria-label="Настройки"
                onClick={() => {
                  void invoke("open_settings");
                }}
                style={{
                  borderColor: `color-mix(in srgb, ${settings.staticGlowColor} 34%, transparent)`,
                  boxShadow: `inset 0 1px 0 0 rgb(255 255 255 / 0.08), 0 0 12px -2px color-mix(in srgb, ${settings.staticGlowColor} 16%, transparent)`,
                  borderRadius: settingsCornerRadiusPx,
                  // Custom property for hover Tailwind arbitrary values below.
                  ["--settings-accent" as string]: settings.staticGlowColor,
                }}
                className="flex h-full w-full items-center justify-center border bg-zinc-950/40 text-zinc-400 transition-[color,background-color,box-shadow,border-color] duration-200 hover:border-[color-mix(in_srgb,var(--settings-accent)_48%,transparent)] hover:bg-zinc-900/55 hover:text-zinc-100 hover:shadow-[inset_0_1px_0_0_rgb(255_255_255/0.12),0_0_16px_0_color-mix(in_srgb,var(--settings-accent)_26%,transparent)]"
              >
                <motion.div
                  className="flex items-center justify-center"
                  style={{ width: settingsIconSizePx, height: settingsIconSizePx }}
                >
                  <Settings className="h-full w-full" strokeWidth={1.75} />
                </motion.div>
              </motion.button>
            </motion.div>
          </motion.div>
          {orientation.ledAxis === "horizontal" ? (
            <span aria-hidden className="h-[3px] w-6 shrink-0" />
          ) : null}
        </motion.div>
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
