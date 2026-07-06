import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { DockIcon } from "./DockIcon";
import { initialApps } from "../lib/mockApps";
import type { DockApp } from "../lib/types";

interface DockCursorPayload {
  x: number;
  y: number;
}

function hitTestIcon(
  refs: Map<string, HTMLButtonElement>,
  x: number,
  y: number,
): string | null {
  for (const [id, el] of refs) {
    const rect = el.getBoundingClientRect();
    if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) {
      return id;
    }
  }
  return null;
}

export function DockPanel() {
  const [apps, setApps] = useState<DockApp[]>(initialApps);
  const [isWindowHovered, setIsWindowHovered] = useState(false);
  const [hoveredIconId, setHoveredIconId] = useState<string | null>(null);

  const iconRefs = useRef<Map<string, HTMLButtonElement>>(new Map());
  const registerIconRef = (id: string, el: HTMLButtonElement | null) => {
    if (el) iconRefs.current.set(id, el);
    else iconRefs.current.delete(id);
  };

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
          setIsWindowHovered(event.payload);
          if (!event.payload) {
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
          const { x, y } = event.payload;
          setHoveredIconId(hitTestIcon(iconRefs.current, x, y));
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
  }, [toggleApp]);

  return (
    // Full-window pointer-events-none shell; pill is interactive. macOS uses a
    // Rust cursor poller for setIgnoreCursorEvents + dock-hover / dock-cursor,
    // and a CGEventTap for dock-click — WKWebView blocks CSS :hover while
    // unfocused, and vibrancy NSVisualEffectView can swallow DOM clicks.
    <div className="pointer-events-none fixed inset-0 z-50 flex flex-col justify-end overflow-hidden pb-2">
      <div className="animate-rgb-glow pointer-events-auto mx-auto flex shrink-0 items-end gap-4 rounded-[28px] border border-transparent bg-zinc-950/80 px-5 py-3 backdrop-blur-xl">
        {apps.map((app) => (
          <DockIcon
            key={app.id}
            app={app}
            registerRef={registerIconRef}
            isHovered={isWindowHovered && hoveredIconId === app.id}
          />
        ))}
      </div>
    </div>
  );
}
