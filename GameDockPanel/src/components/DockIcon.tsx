import { useState } from "react";
import type { DockApp } from "../lib/types";

interface DockIconProps {
  app: DockApp;
  onToggle: (id: string) => void;
  registerRef?: (id: string, el: HTMLButtonElement | null) => void;
}

export function DockIcon({ app, onToggle, registerRef }: DockIconProps) {
  const [broken, setBroken] = useState(false);

  const iconVisualClass =
    "h-14 w-14 origin-bottom rounded-2xl shadow-lg shadow-black/40 transition-transform duration-300 ease-out group-hover:scale-120";

  return (
    <button
      type="button"
      ref={(el) => registerRef?.(app.id, el)}
      onClick={() => onToggle(app.id)}
      aria-pressed={app.isActive}
      aria-label={`${app.name}${app.isActive ? " (running)" : ""}`}
      className="group relative flex flex-col items-center gap-2 outline-none"
    >
      {/* Tooltip sits just above the icon — less vertical overflow than -top-9 */}
      <div className="relative shrink-0">
        <span className="pointer-events-none absolute bottom-full left-1/2 mb-1 -translate-x-1/2 scale-90 whitespace-nowrap rounded-md bg-zinc-900/90 px-2 py-1 text-xs text-zinc-200 opacity-0 shadow-lg shadow-black/40 transition-all duration-300 ease-out group-hover:scale-100 group-hover:opacity-100">
          {app.name}
        </span>

        {broken ? (
          <div
            className={`flex items-center justify-center bg-zinc-800 text-lg font-semibold ${iconVisualClass} ${app.color}`}
          >
            {app.name.slice(0, 2).toUpperCase()}
          </div>
        ) : (
          <img
            src={app.iconUrl}
            alt={app.name}
            draggable={false}
            onError={() => setBroken(true)}
            className={`object-cover ${iconVisualClass}`}
          />
        )}
      </div>

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
