import { useState } from "react";
import type { DockApp } from "../lib/types";

interface DockIconProps {
  app: DockApp;
  onToggle: (id: string) => void;
  /**
   * Extension point for the future hover-magnify pass: lets the parent keep
   * a DOM-ref per icon (keyed by id) to measure cursor-to-icon distance on
   * mousemove. Unused beyond storage for now.
   */
  registerRef?: (id: string, el: HTMLButtonElement | null) => void;
}

export function DockIcon({ app, onToggle, registerRef }: DockIconProps) {
  // Tracks whether the remote icon failed to load so we can swap in an
  // initials badge instead of a broken-image glyph.
  const [broken, setBroken] = useState(false);

  return (
    <button
      type="button"
      ref={(el) => registerRef?.(app.id, el)}
      onClick={() => onToggle(app.id)}
      aria-pressed={app.isActive}
      aria-label={`${app.name}${app.isActive ? " (running)" : ""}`}
      className="group relative flex flex-col items-center gap-2 outline-none"
    >
      {/* macOS-style name tooltip, ready for the future hover polish */}
      <span className="pointer-events-none absolute -top-9 scale-90 whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 opacity-0 shadow-lg shadow-black/40 transition-all duration-300 ease-out group-hover:scale-100 group-hover:opacity-100">
        {app.name}
      </span>

      {broken ? (
        <div
          className={`flex h-14 w-14 items-center justify-center rounded-2xl bg-zinc-800 text-lg font-semibold shadow-lg shadow-black/40 transition-all duration-300 ease-out group-hover:scale-120 ${app.color}`}
        >
          {app.name.slice(0, 2).toUpperCase()}
        </div>
      ) : (
        <img
          src={app.iconUrl}
          alt={app.name}
          draggable={false}
          onError={() => setBroken(true)}
          className="h-14 w-14 rounded-2xl object-cover shadow-lg shadow-black/40 transition-all duration-300 ease-out group-hover:scale-120"
        />
      )}

      {/* Running-app LED: bg-current + the app's text-* color means the dot
          and its breathing glow always match, no extra classes to keep in
          sync. Stays mounted (just scaled to 0) so a future
          hover:scale-120 on the icon can drag this along smoothly. */}
      <span
        className={`h-[3px] w-6 rounded-full bg-current transition-all duration-300 ease-out group-hover:scale-120 ${
          app.isActive
            ? `${app.color} animate-led-pulse opacity-100`
            : "scale-0 text-transparent opacity-0"
        }`}
      />
    </button>
  );
}
