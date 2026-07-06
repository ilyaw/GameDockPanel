import { useState } from "react";
import type { DockApp } from "../lib/types";

interface DockIconProps {
  app: DockApp;
  registerRef?: (id: string, el: HTMLButtonElement | null) => void;
  /** Driven by Rust dock-cursor events — CSS :hover is blocked when unfocused. */
  isHovered?: boolean;
}

export function DockIcon({
  app,
  registerRef,
  isHovered = false,
}: DockIconProps) {
  const [broken, setBroken] = useState(false);

  const iconVisualClass =
    "h-14 w-14 origin-bottom rounded-2xl shadow-lg shadow-black/40 transition-transform duration-300 ease-out";
  const hoveredScale = isHovered ? "scale-120" : "";

  return (
    <button
      type="button"
      ref={(el) => registerRef?.(app.id, el)}
      aria-pressed={app.isActive}
      aria-label={`${app.name}${app.isActive ? " (running)" : ""}`}
      className="relative flex flex-col items-center gap-2 outline-none"
    >
      <div className="relative shrink-0">
        <span
          className={`pointer-events-none absolute bottom-full left-1/2 mb-0.5 -translate-x-1/2 whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 shadow-lg shadow-black/40 transition-all duration-300 ease-out ${
            isHovered
              ? "scale-100 opacity-100"
              : "scale-90 opacity-0"
          }`}
        >
          {app.name}
        </span>

        {broken ? (
          <div
            className={`flex items-center justify-center bg-zinc-800 text-lg font-semibold ${iconVisualClass} ${hoveredScale} ${app.color}`}
          >
            {app.name.slice(0, 2).toUpperCase()}
          </div>
        ) : (
          <img
            src={app.iconUrl}
            alt={app.name}
            draggable={false}
            onError={() => setBroken(true)}
            className={`object-cover ${iconVisualClass} ${hoveredScale}`}
          />
        )}
      </div>

      <span
        className={`h-[3px] w-6 rounded-full bg-current transition-all duration-300 ease-out ${hoveredScale} ${
          app.isActive
            ? `${app.color} animate-led-pulse opacity-100`
            : "scale-0 text-transparent opacity-0"
        }`}
      />
    </button>
  );
}
