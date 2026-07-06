import { useCallback, useEffect, useRef, useState } from "react";
import { useMotionValue } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { DockIcon } from "./DockIcon";
import { PILL_HEIGHT_PX } from "../lib/constants";
import { initialApps } from "../lib/mockApps";
import type { DockApp } from "../lib/types";

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
  const [apps, setApps] = useState<DockApp[]>(initialApps);
  const [hoveredIconId, setHoveredIconId] = useState<string | null>(null);

  const mouseX = useMotionValue(Infinity);
  const lastNativeMoveAt = useRef(0);
  const dockHoveredRef = useRef(false);

  const iconRefs = useRef<Map<string, HTMLElement>>(new Map());
  const registerIconRef = (id: string, el: HTMLElement | null) => {
    if (el) iconRefs.current.set(id, el);
    else iconRefs.current.delete(id);
  };

  const applyCursor = useCallback((x: number, y: number) => {
    mouseX.set(x);
    setHoveredIconId(hitTestIcon(iconRefs.current, x, y));
  }, [mouseX]);

  const leaveDock = useCallback(() => {
    dockHoveredRef.current = false;
    mouseX.set(Infinity);
    setHoveredIconId(null);
  }, [mouseX]);

  const toggleApp = useCallback((id: string) => {
    setApps((prev) =>
      prev.map((app) =>
        app.id === id ? { ...app, isActive: !app.isActive } : app,
      ),
    );
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void (async () => {
      unlisteners.push(
        await listen<boolean>("dock-hover", (event) => {
          dockHoveredRef.current = event.payload;
          if (!event.payload) {
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
          const { x, y } = event.payload;
          const id = hitTestIcon(iconRefs.current, x, y);
          if (id) toggleApp(id);
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
  }, [toggleApp, mouseX, applyCursor]);

  return (
    <div className="pointer-events-none fixed inset-0 z-50 flex flex-col justify-end overflow-visible pb-2">
      <div
        onMouseEnter={() => {
          dockHoveredRef.current = true;
        }}
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
        className="animate-rgb-glow pointer-events-auto mx-auto flex shrink-0 items-end gap-5 overflow-visible rounded-[28px] border border-transparent bg-zinc-950/80 px-5 py-3"
      >
        {apps.map((app) => (
          <DockIcon
            key={app.id}
            app={app}
            registerRef={registerIconRef}
            mouseX={mouseX}
            isHovered={hoveredIconId === app.id}
          />
        ))}
      </div>
    </div>
  );
}
