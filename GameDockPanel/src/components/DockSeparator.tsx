import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { type MotionValue } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Trash2 } from "lucide-react";
import { DockRowDivider } from "./DockRowDivider";
import { DockOverlayAnchor } from "./DockOverlayAnchor";
import { DOCK_SEPARATOR_WIDTH_PX, TOOLTIP_GAP_PX } from "../lib/constants";
import {
  resolveOverlaySide,
  type OverlaySide,
} from "../lib/dockPlacement";

interface WindowLogicalPoint {
  x: number;
  y: number;
}

interface DockSeparatorProps {
  id: string;
  iconSizePx: MotionValue<number>;
  isVertical?: boolean;
  overlayPreferredSide: OverlaySide;
  onRemove?: (separatorId: string) => void;
  isDragging?: boolean;
  contextMenuActive?: boolean;
  onContextMenuOpenChange?: (open: boolean) => void;
}

export function DockSeparator({
  id,
  iconSizePx,
  isVertical = false,
  overlayPreferredSide,
  onRemove,
  isDragging = false,
  onContextMenuOpenChange,
}: DockSeparatorProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuSide, setMenuSide] = useState<OverlaySide>(overlayPreferredSide);
  const anchorRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    onContextMenuOpenChange?.(menuOpen);
    return () => {
      if (menuOpen) onContextMenuOpenChange?.(false);
    };
  }, [menuOpen, onContextMenuOpenChange]);

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
    const reportMenuOverlay = (
      active: boolean,
      side: OverlaySide,
      width: number,
      height: number,
    ) => {
      invoke("set_menu_overlay", { active, side, width, height }).catch(
        (error: unknown) => {
          console.error("Failed to sync menu overlay hit-test region:", error);
        },
      );
    };

    if (!menuOpen) {
      reportMenuOverlay(false, overlayPreferredSide, 0, 0);
      return;
    }

    const measure = () => {
      const anchorEl = anchorRef.current;
      const menuEl = menuRef.current;
      if (!anchorEl || !menuEl) return;

      const anchorRect = anchorEl.getBoundingClientRect();
      const menuRect = menuEl.getBoundingClientRect();
      const resolvedSide = resolveOverlaySide(
        anchorRect,
        { width: menuRect.width, height: menuRect.height },
        overlayPreferredSide,
        TOOLTIP_GAP_PX,
      );
      setMenuSide(resolvedSide);
      reportMenuOverlay(true, resolvedSide, menuRect.width, menuRect.height);
    };

    measure();
    const menuEl = menuRef.current;
    if (!menuEl) return;
    const observer = new ResizeObserver(measure);
    observer.observe(menuEl);
    return () => {
      observer.disconnect();
      reportMenuOverlay(false, overlayPreferredSide, 0, 0);
    };
  }, [menuOpen, overlayPreferredSide]);

  return (
    <div
      ref={anchorRef}
      role="separator"
      aria-orientation={isVertical ? "horizontal" : "vertical"}
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
      style={isVertical ? { height: DOCK_SEPARATOR_WIDTH_PX } : { width: DOCK_SEPARATOR_WIDTH_PX }}
      className={`relative flex shrink-0 outline-none ${
        isVertical
          ? "items-center justify-center self-center"
          : "items-end justify-center self-end"
      } ${isDragging ? "cursor-grabbing" : "cursor-grab"} ${menuOpen ? "z-10" : "z-0"}`}
    >
      {menuOpen && (
        <DockOverlayAnchor
          innerRef={menuRef}
          side={menuSide}
          gap={TOOLTIP_GAP_PX}
          className="pointer-events-auto z-30 overflow-hidden whitespace-nowrap rounded-md bg-zinc-900/95 text-xs text-zinc-200 shadow-lg shadow-black/40"
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
        </DockOverlayAnchor>
      )}

      <DockRowDivider iconSizePx={iconSizePx} isVertical={isVertical} />
    </div>
  );
}
