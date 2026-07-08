import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Reorder, motion, useMotionValue, useSpring, useTransform } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Settings } from "lucide-react";
import { DockIcon } from "./DockIcon";
import { DockRowDivider } from "./DockRowDivider";
import { DockSeparator } from "./DockSeparator";
import { useDockApps } from "../hooks/useDockApps";
import { useDockSettings } from "../hooks/useDockSettings";
import {
  MAGNIFY_MAX_SCALE,
  MAX_SEPARATORS,
  TOOLTIP_GAP_PX,
  BG_ANIMATION_CLASSES,
  FLOW_RING_ANIMATION_CLASSES,
  getBackgroundPreset,
  backgroundPresetToDurationS,
  getBorderStylePreset,
  getFlowRingVariant,
  getPanelEffectPreset,
  getSizeMetrics,
} from "../lib/constants";
import type { DockItem } from "../lib/types";
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

/** macOS Dock-style slot: insert before the first app icon whose center is
 * right of the cursor; otherwise append before settings. */
function resolveInsertIndex(
  items: DockItem[],
  refs: Map<string, HTMLElement>,
  pillEl: HTMLElement | null,
  x: number,
  y: number,
): number {
  if (pillEl && !pointInRect(x, y, pillEl)) {
    return items.length;
  }

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (!isDockAppItem(item)) continue;
    const el = refs.get(item.id);
    if (!el) continue;
    const rect = el.getBoundingClientRect();
    const centerX = rect.left + rect.width / 2;
    if (x < centerX) return i;
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

function getInsertMarkerMetrics(
  items: DockItem[],
  refs: Map<string, HTMLElement>,
  pillEl: HTMLElement,
  insertIndex: number,
): { left: number; top: number; height: number } | null {
  const pillRect = pillEl.getBoundingClientRect();
  let markerX: number | null = null;
  let markerTop = 0;
  let markerHeight = 0;

  if (insertIndex <= 0) {
    const firstId = findAppIdAtOrAfter(items, 0);
    const first = firstId ? refs.get(firstId) : null;
    if (first) {
      const rect = first.getBoundingClientRect();
      markerX = rect.left;
      markerTop = rect.top;
      markerHeight = rect.height;
    }
  } else if (insertIndex >= items.length) {
    const lastId = findAppIdAtOrBefore(items, items.length - 1);
    const last = lastId ? refs.get(lastId) : null;
    if (last) {
      const rect = last.getBoundingClientRect();
      markerX = rect.right;
      markerTop = rect.top;
      markerHeight = rect.height;
    }
  } else {
    const prevId = findAppIdAtOrBefore(items, insertIndex - 1);
    const nextId = findAppIdAtOrAfter(items, insertIndex);
    const prev = prevId ? refs.get(prevId) : null;
    const next = nextId ? refs.get(nextId) : null;
    if (prev && next) {
      const prevRect = prev.getBoundingClientRect();
      const nextRect = next.getBoundingClientRect();
      markerX = (prevRect.right + nextRect.left) / 2;
      markerTop = Math.min(prevRect.top, nextRect.top);
      markerHeight = Math.max(prevRect.height, nextRect.height);
    } else if (prev) {
      const prevRect = prev.getBoundingClientRect();
      markerX = prevRect.right;
      markerTop = prevRect.top;
      markerHeight = prevRect.height;
    } else if (next) {
      const nextRect = next.getBoundingClientRect();
      markerX = nextRect.left;
      markerTop = nextRect.top;
      markerHeight = nextRect.height;
    }
  }

  if (markerX === null || markerHeight < 1) return null;
  return {
    left: markerX - pillRect.left,
    top: markerTop - pillRect.top,
    height: markerHeight,
  };
}

export function DockPanel() {
  const {
    items,
    activateApp,
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
  } = useDockApps();
  const separatorsFull = countDockSeparators(items) >= MAX_SEPARATORS;
  const { settings, hydrated } = useDockSettings();
  const iconSizeTarget = useMotionValue(settings.iconSizePx);
  const iconSizeAnimated = useSpring(iconSizeTarget, ICON_SIZE_SPRING);
  const iconSizeSyncedRef = useRef(false);
  const geometrySyncRafRef = useRef(0);
  /** Static layout numbers for the first paint — guarantees a non-zero pill
   * rect before Motion values land in the DOM. */
  const restMetrics = useMemo(
    () => getSizeMetrics(settings.iconSizePx),
    [settings.iconSizePx],
  );

  useEffect(() => {
    if (!hydrated) return;

    if (!iconSizeSyncedRef.current) {
      iconSizeTarget.jump(settings.iconSizePx);
      iconSizeAnimated.jump(settings.iconSizePx);
      iconSizeSyncedRef.current = true;
      return;
    }

    iconSizeTarget.set(settings.iconSizePx);
  }, [settings.iconSizePx, hydrated, iconSizeTarget, iconSizeAnimated]);

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

  const pillHeightPx = useTransform(iconSizeAnimated, (px) => getSizeMetrics(px).pillHeightPx);
  const pillGapPx = useTransform(iconSizeAnimated, (px) => getSizeMetrics(px).dockGapPx);
  const pillPaddingInlinePx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).dockPaddingXPx,
  );
  const pillPaddingBlockPx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).dockPaddingYPx,
  );
  const iconRowGapPx = useTransform(iconSizeAnimated, (px) => getSizeMetrics(px).dockGapPx);
  const settingsSlotSizePx = useTransform(iconSizeAnimated, (px) => px);
  const settingsCornerRadiusPx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).iconCornerRadiusPx,
  );
  const settingsMagnifyRadiusPx = useTransform(
    iconSizeAnimated,
    (px) => getSizeMetrics(px).magnifyInfluenceRadiusPx,
  );
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
  const [insertMarker, setInsertMarker] = useState<{
    left: number;
    top: number;
    height: number;
  } | null>(null);

  const mouseX = useMotionValue(Infinity);
  /** Separate cursor channel for the settings slot — when the pointer is over
   * settings, `mouseX` is forced to `Infinity` so app icons don't pick up a
   * distant magnify curve from the far-right cursor X (which was making the
   * leftmost icon grow). Settings reads this value instead. */
  const settingsMouseX = useMotionValue(Infinity);
  const lastNativeMoveAt = useRef(0);
  const dockHoveredRef = useRef(false);

  const iconRefs = useRef<Map<string, HTMLElement>>(new Map());
  const pillRef = useRef<HTMLDivElement | null>(null);
  const settingsSlotRef = useRef<HTMLDivElement>(null);
  const settingsCenterX = useMotionValue(0);
  const registerIconRef = (id: string, el: HTMLElement | null) => {
    if (el) iconRefs.current.set(id, el);
    else iconRefs.current.delete(id);
  };

  useEffect(() => {
    resolveInsertIndexRef.current = (x, y) =>
      resolveInsertIndex(items, iconRefs.current, pillRef.current, x, y);
  }, [items, resolveInsertIndexRef]);

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
    );
    setInsertMarker(metrics);
  }, [items, fileDragOver, fileDragInsertIndex, iconSizeAnimated]);

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

  const applyCursor = useCallback(
    (x: number, y: number) => {
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

  const beginDrag = useCallback(() => {
    orderAtDragStartRef.current = items.map((item) => item.id);
    setIsDragging(true);
    mouseX.set(Infinity);
    settingsMouseX.set(Infinity);
    setHoveredIconId(null);
    setIsSettingsHovered(false);
  }, [items, mouseX, settingsMouseX]);

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

    scheduleGeometrySync();

    let lastPillWidth = measurePill().width;
    let lastPillHeight = measurePill().height;
    let lastPillTop = measurePill().top;
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
      unsubscribeIconSize();
      if (geometrySyncRafRef.current) {
        cancelAnimationFrame(geometrySyncRafRef.current);
        geometrySyncRafRef.current = 0;
      }
      observer.disconnect();
    };
  }, [items, iconSizeAnimated]);

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
  }, [settingsCenterX, items, hoverSessionId, reorderSettledId]);

  const settingsScaleRaw = useTransform(
    [settingsMouseX, settingsCenterX, settingsMagnifyRadiusPx],
    ([mx, cx, radius]: number[]) => {
      if (!Number.isFinite(mx)) return 1;
      const distance = mx - cx;
      const t = Math.abs(distance) / radius;
      if (t >= 1) return 1;
      return 1 + (MAGNIFY_MAX_SCALE - 1) * (1 - t);
    },
  );
  const settingsScale = useSpring(settingsScaleRaw, MAGNIFY_SPRING);

  const activeBorderStyle = getBorderStylePreset(settings.borderStyle);
  const activeBgPreset = getBackgroundPreset(settings.backgroundPreset);
  const flowRingVariant = getFlowRingVariant(activeBorderStyle.id);
  /**
   * "Scan" doesn't animate `border-color`/`box-shadow` like the other three
   * styles — it gets a dedicated conic-gradient ring overlay instead (see
   * `.dock-border-scan-ring` in index.css). Spotlight-style `flow-*` styles
   * use `.dock-border-flow-ring` the same way. Suppressed during a reject
   * flash or an active file drag-over, same as the other border styles,
   * since both of those already communicate their own state through the
   * pill's real border.
   */
  const showScanRing =
    settings.animationsEnabled &&
    activeBorderStyle.id === "scan" &&
    !isRejecting &&
    !fileDragOver;

  const showFlowRing =
    settings.animationsEnabled &&
    flowRingVariant !== null &&
    !isRejecting &&
    !fileDragOver;

  const showGradientRing = showScanRing || showFlowRing;

  const bgAnimClasses = BG_ANIMATION_CLASSES[activeBgPreset.animation];

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
    const preset = activeBgPreset;
    const mixed = preset.colors.map(
      (color) =>
        `color-mix(in srgb, ${color} ${Math.round(settings.backgroundIntensity * 100)}%, black)`,
    );
    return {
      borderColor: showGradientRing ? "transparent" : settings.staticGlowColor,
      boxShadow: showGradientRing
        ? "none"
        : `0 0 14px 0 color-mix(in srgb, ${settings.staticGlowColor} 40%, transparent)`,
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
    settings.staticGlowColor,
    settings.rgbGlowColors,
    settings.backgroundIntensity,
    settings.backgroundSpeed,
    showGradientRing,
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
      <motion.div
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
        style={{
          ...pillStyle,
          height: pillHeightPx,
          gap: pillGapPx,
          paddingInline: pillPaddingInlinePx,
          paddingBlock: pillPaddingBlockPx,
          // Static fallback until Motion values commit on the first frame.
          minHeight: restMetrics.pillHeightPx,
        }}
        className={`pointer-events-auto relative mx-auto m-0 flex shrink-0 items-end overflow-visible rounded-[28px] border transition-colors ${
          isRejecting
            ? "animate-reject-pulse"
            : settings.animationsEnabled && !showGradientRing
              ? activeBorderStyle.animationClass
              : ""
        } ${
          fileDragOver
            ? "border-zinc-400 bg-zinc-900/90"
            : "border-transparent bg-black/40"
        }`}
      >
        {(showScanRing || showFlowRing) && (
          // Clip gradient rings to the pill footprint — conic-gradient fills
          // a rectangular box and WebKit's mask-composite ring trick can leak
          // square corners without an overflow-hidden rounded wrapper.
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0 -z-10 overflow-hidden rounded-[28px]"
          >
            {showScanRing && (
              // Separate overlay, not a class on the pill itself — see the
              // "scan" branch note on `showScanRing`/`activeBorderStyle` above
              // for why a rotating gradient can't just replace `border-color`
              // like the other three styles do.
              <div
                aria-hidden
                className="dock-border-scan-ring animate-border-scan-rotate pointer-events-none absolute -inset-px rounded-[28px]"
              />
            )}
            {showFlowRing && flowRingVariant && (
              <div
                aria-hidden
                className={`dock-border-flow-ring ${FLOW_RING_ANIMATION_CLASSES[flowRingVariant]} pointer-events-none absolute -inset-px rounded-[28px]`}
              />
            )}
          </div>
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
              className={`${bgAnimClasses} absolute -inset-8 blur-2xl`}
              style={bgFlowStyle}
            />
          </div>
        )}
        {insertMarker && (
          <div
            aria-hidden
            className="pointer-events-none absolute z-30 w-0.5 -translate-x-1/2 rounded-full bg-zinc-200/90 shadow-[0_0_8px_2px_rgb(255_255_255/0.35)]"
            style={{
              left: insertMarker.left,
              top: insertMarker.top,
              height: insertMarker.height,
            }}
          />
        )}
        <Reorder.Group
          axis="x"
          values={items}
          onReorder={handleReorder}
          style={{ gap: iconRowGapPx }}
          className="m-0 flex list-none items-end"
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
                registerRef={registerIconRef}
                mouseX={mouseX}
                isHovered={
                  !isDragging && !isReorderSettling && hoveredIconId === item.id
                }
                hoverSessionId={hoverSessionId}
                reorderSettledId={reorderSettledId}
                isDragging={isDragging}
                isReorderSettling={isReorderSettling}
                animationsEnabled={settings.animationsEnabled}
                isBouncing={bouncingIds.has(item.id)}
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
                onRemove={removeSeparator}
                isDragging={isDragging}
              />
            )}
          </Reorder.Item>
        ))}
        </Reorder.Group>
        <DockRowDivider iconSizePx={iconSizeAnimated} className="mx-1" />
        {/* Matches DockIcon's own `flex-col items-center gap-2` shape (icon +
            gap + LED) with an invisible spacer standing in for the LED — the
            pill row is `items-end`, so without this the button's bottom
            (and thus its glyph) lands 11px lower than every app icon's. */}
        <div className="flex shrink-0 flex-col items-center gap-2">
          <motion.div
            ref={settingsSlotRef}
            style={{ height: settingsSlotSizePx, width: settingsSlotSizePx }}
            className={`relative shrink-0 ${
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
