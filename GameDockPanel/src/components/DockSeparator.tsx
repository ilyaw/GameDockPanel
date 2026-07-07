import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { type MotionValue } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Trash2 } from "lucide-react";
import { DockRowDivider } from "./DockRowDivider";
import { DOCK_SEPARATOR_WIDTH_PX, TOOLTIP_GAP_PX } from "../lib/constants";

interface WindowLogicalPoint {
  x: number;
  y: number;
}

interface DockSeparatorProps {
  id: string;
  iconSizePx: MotionValue<number>;
  onRemove?: (separatorId: string) => void;
  isDragging?: boolean;
}

export function DockSeparator({
  id,
  iconSizePx,
  onRemove,
  isDragging = false,
}: DockSeparatorProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };

    const handlePointerDown = (event: MouseEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      setMenuOpen(false);
    };

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [menuOpen]);

  useEffect(() => {
    if (!menuOpen) return;

    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void listen<WindowLogicalPoint>("dock-global-mousedown", (event) => {
      const rect = menuRef.current?.getBoundingClientRect();
      if (!rect) return;
      const { x, y } = event.payload;
      const insideMenu =
        x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
      if (!insideMenu) setMenuOpen(false);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [menuOpen]);

  useLayoutEffect(() => {
    const reportMenuOverlay = (active: boolean, height: number) => {
      invoke("set_menu_overlay", { active, height }).catch((error: unknown) => {
        console.error("Failed to sync menu overlay hit-test region:", error);
      });
    };

    if (!menuOpen) {
      reportMenuOverlay(false, 0);
      return;
    }

    const measure = () => {
      reportMenuOverlay(true, menuRef.current?.getBoundingClientRect().height ?? 0);
    };

    measure();
    const menuEl = menuRef.current;
    if (!menuEl) return;
    const observer = new ResizeObserver(measure);
    observer.observe(menuEl);
    return () => {
      observer.disconnect();
      reportMenuOverlay(false, 0);
    };
  }, [menuOpen]);

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
      style={{ width: DOCK_SEPARATOR_WIDTH_PX }}
      className={`relative flex shrink-0 items-end justify-center self-end outline-none ${
        isDragging ? "cursor-grabbing" : "cursor-grab"
      } ${menuOpen ? "z-10" : "z-0"}`}
    >
      {menuOpen && (
        <div
          ref={menuRef}
          style={{ marginBottom: TOOLTIP_GAP_PX }}
          className="pointer-events-auto absolute bottom-full left-1/2 z-30 -translate-x-1/2 overflow-hidden whitespace-nowrap rounded-md bg-zinc-900/95 text-xs text-zinc-200 shadow-lg shadow-black/40"
        >
          <button
            type="button"
            onClick={() => {
              setMenuOpen(false);
              onRemove?.(id);
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-zinc-800"
          >
            <Trash2 className="h-3.5 w-3.5" />
            Удалить разделитель
          </button>
        </div>
      )}

      <DockRowDivider iconSizePx={iconSizePx} />
    </div>
  );
}
