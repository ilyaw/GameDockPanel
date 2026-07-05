import { useEffect, useRef, useState } from "react";
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
      }
    })();

    return () => {
      cancelled = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, []);

  const toggleApp = (id: string) => {
    setApps((prev) =>
      prev.map((app) =>
        app.id === id ? { ...app, isActive: !app.isActive } : app,
      ),
    );
  };

  return (
    // Full-window pointer-events-none shell; pill is interactive. macOS also
    // uses a Rust cursor poller (platform/macos.rs) that toggles
    // setIgnoreCursorEvents and emits dock-hover / dock-cursor — WKWebView
    // blocks CSS :hover while the window is unfocused.
    <div className="pointer-events-none fixed inset-0 z-50 flex flex-col justify-end overflow-hidden pb-2">
      <div className="animate-rgb-glow pointer-events-auto mx-auto flex shrink-0 items-end gap-4 rounded-[28px] border border-transparent bg-zinc-950/80 px-5 py-3 backdrop-blur-xl">
        {apps.map((app) => (
          <DockIcon
            key={app.id}
            app={app}
            onToggle={toggleApp}
            registerRef={registerIconRef}
            isHovered={isWindowHovered && hoveredIconId === app.id}
          />
        ))}
      </div>
    </div>
  );
}
