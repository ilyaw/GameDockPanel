import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Reorder, useMotionValue } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { DockIcon } from "./DockIcon";
import { useDockApps } from "../hooks/useDockApps";
import { PILL_HEIGHT_PX } from "../lib/constants";
import type { DockApp } from "../lib/types";

/** How long the pill's reject-pulse border stays applied — mirrors the
 * `--animate-reject-pulse` duration in `index.css`; kept here instead of
 * imported since it's a one-shot JS timer, not a CSS-consumed constant. */
const REJECT_PULSE_MS = 400;

interface DockCursorPayload {
  x: number;
  y: number;
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
    removeApp,
    fileDragOver,
    rejectPulseKey,
    showInFinder,
    quitApp,
  } = useDockApps();
  const [hoveredIconId, setHoveredIconId] = useState<string | null>(null);
  const [hoverSessionId, setHoverSessionId] = useState(0);
  const [isDragging, setIsDragging] = useState(false);
  const isDraggingRef = useRef(false);
  const [isRejecting, setIsRejecting] = useState(false);

  const mouseX = useMotionValue(Infinity);
  const lastNativeMoveAt = useRef(0);
  const dockHoveredRef = useRef(false);

  const iconRefs = useRef<Map<string, HTMLElement>>(new Map());
  const pillRef = useRef<HTMLUListElement>(null);
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

  const handleReorder = useCallback(
    (newOrder: DockApp[]) => {
      void reorderApps(newOrder);
    },
    [reorderApps],
  );

  const beginDrag = useCallback(() => {
    setIsDragging(true);
    mouseX.set(Infinity);
    setHoveredIconId(null);
  }, [mouseX]);

  const endDrag = useCallback(() => {
    setIsDragging(false);
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

  return (
    <div className="pointer-events-none fixed inset-0 z-50 flex flex-col justify-end overflow-visible pb-2">
      <Reorder.Group
        ref={pillRef}
        axis="x"
        values={apps}
        onReorder={handleReorder}
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
        style={{ height: PILL_HEIGHT_PX }}
        className={`pointer-events-auto mx-auto m-0 flex shrink-0 list-none items-end gap-2 overflow-visible rounded-[28px] border px-5 py-3 transition-colors ${
          isRejecting ? "animate-reject-pulse" : "animate-rgb-glow"
        } ${
          fileDragOver
            ? "border-zinc-400 bg-zinc-900/90"
            : "border-transparent bg-zinc-950/80"
        }`}
      >
        {apps.map((app) => (
          <Reorder.Item
            key={app.id}
            value={app}
            className="relative list-none"
            whileDrag={{ zIndex: 20, scale: 1.08 }}
            onDragStart={beginDrag}
            onDragEnd={endDrag}
          >
            <DockIcon
              app={app}
              registerRef={registerIconRef}
              mouseX={mouseX}
              isHovered={!isDragging && hoveredIconId === app.id}
              hoverSessionId={hoverSessionId}
              isDragging={isDragging}
              onRemove={removeApp}
              onShowInFinder={showInFinder}
              onQuit={quitApp}
            />
          </Reorder.Item>
        ))}
      </Reorder.Group>
    </div>
  );
}
