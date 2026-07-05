import { useRef, useState } from "react";
import { DockIcon } from "./DockIcon";
import { initialApps } from "../lib/mockApps";
import type { DockApp } from "../lib/types";

export function DockPanel() {
  const [apps, setApps] = useState<DockApp[]>(initialApps);

  // One DOM ref per icon, keyed by app id — groundwork for the future
  // hover-magnify pass (cursor-to-icon distance needs each icon's rect).
  // Not read anywhere yet.
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
    // Outer strip is click-through (pointer-events-none) so the transparent,
    // always-on-top Tauri window doesn't block clicks on whatever sits
    // underneath it — only the dock pill itself is interactive.
    //
    // NOTE: this is CSS-only click-through for now. The `setIgnoreCursorEvents`
    // second layer from the tauri-glass-dock skill is intentionally NOT wired
    // up here — a naive mouseenter/mouseleave toggle deadlocks the window
    // (once ignoreCursorEvents(true) is set, the webview stops receiving ANY
    // mouse events, including the mouseenter needed to turn it back off; see
    // tauri-apps/tauri#2090, #6164, #13070). A real fix needs a Rust-side
    // global cursor poller reporting against a hitbox — deferred to the
    // hover-magnify pass, which needs live cursor tracking anyway. The Tauri
    // window is sized tightly around this pill (see platform/macos.rs) so the
    // non-pill "dead click zone" left by skipping that second layer stays a
    // thin margin, not a screen-wide strip.
    <div className="pointer-events-none fixed inset-x-0 bottom-5 z-50 flex justify-center">
      <div className="animate-rgb-glow pointer-events-auto flex items-end gap-4 rounded-[28px] border border-transparent bg-zinc-950/80 px-5 py-3.5 backdrop-blur-xl">
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
