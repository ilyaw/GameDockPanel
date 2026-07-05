import { useRef, useState } from "react";
import { DockIcon } from "./DockIcon";
import { initialApps } from "../lib/mockApps";
import type { DockApp } from "../lib/types";

export function DockPanel() {
  const [apps, setApps] = useState<DockApp[]>(initialApps);

  const iconRefs = useRef<Map<string, HTMLButtonElement>>(new Map());
  const registerIconRef = (id: string, el: HTMLButtonElement | null) => {
    if (el) iconRefs.current.set(id, el);
    else iconRefs.current.delete(id);
  };

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
    // setIgnoreCursorEvents so transparent bands above the pill never steal
    // focus from apps underneath — CSS alone is not enough on WKWebView.
    <div className="pointer-events-none fixed inset-0 z-50 flex flex-col justify-end overflow-hidden pb-2">
      <div className="animate-rgb-glow pointer-events-auto mx-auto flex shrink-0 items-end gap-4 rounded-[28px] border border-transparent bg-zinc-950/80 px-5 py-3 backdrop-blur-xl">
        {apps.map((app) => (
          <DockIcon
            key={app.id}
            app={app}
            onToggle={toggleApp}
            registerRef={registerIconRef}
          />
        ))}
      </div>
    </div>
  );
}
