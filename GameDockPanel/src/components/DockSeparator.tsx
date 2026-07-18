import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { type MotionValue } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Trash2 } from "lucide-react";
import { DockContextMenuRow } from "./DockContextMenuRow";
import { DockRowDivider } from "./DockRowDivider";
import { DockOverlayAnchor } from "./DockOverlayAnchor";
import {
  DOCK_SEPARATOR_HIT_PX,
  DOCK_SEPARATOR_WIDTH_PX,
  TOOLTIP_GAP_PX,
} from "../lib/constants";
import {
  resolveOverlaySide,
  type OverlaySide,
} from "../lib/dockPlacement";
import { setDockRegionRelaxed } from "../lib/windowsDock";
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
  onContextMenuOpenChange?: (open: boolean) => void;
  onContextMenuBoundsChange?: (rect: DOMRect | null) => void;
}

export function DockSeparator({
  id,
  iconSizePx,
  isVertical = false,
  overlayPreferredSide,
  onRemove,
  isDragging = false,
  onContextMenuOpenChange,
  onContextMenuBoundsChange,
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
      onContextMenuBoundsChange?.(null);
      return;
    }

    const measure = () => {
      const anchorEl = anchorRef.current;
      const menuEl = menuRef.current;
      if (!anchorEl || !menuEl) return;

      const anchorRect = anchorEl.getBoundingClientRect();
      const menuRect = menuEl.getBoundingClientRect();
      onContextMenuBoundsChange?.(menuRect);
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
      onContextMenuBoundsChange?.(null);
    };
  }, [menuOpen, overlayPreferredSide, onContextMenuBoundsChange]);

  return (
    <div
      data-dock-item
      role="separator"
      aria-orientation={isVertical ? "horizontal" : "vertical"}
      style={
        isVertical
          ? { height: DOCK_SEPARATOR_WIDTH_PX }
          : { width: DOCK_SEPARATOR_WIDTH_PX }
      }
      className={`relative shrink-0 ${
        isVertical ? "self-center" : "self-end"
      }`}
    >
      <div
        onContextMenu={(event) => {
          event.preventDefault();
          void setDockRegionRelaxed(true, { menuHold: true }).then(() => setMenuOpen(true));
        }}
        style={
          isVertical
            ? { height: DOCK_SEPARATOR_HIT_PX, width: "100%" }
            : { width: DOCK_SEPARATOR_HIT_PX, height: "100%" }
        }
        className={`absolute left-1/2 top-1/2 flex -translate-x-1/2 -translate-y-1/2 outline-none ${
          isVertical
            ? "items-center justify-center"
            : "items-end justify-center"
        } ${isDragging ? "cursor-grabbing" : "cursor-grab"} ${
          menuOpen ? "z-[30]" : "z-[11]"
        }`}
      >
        {menuOpen && (
          <DockOverlayAnchor
            innerRef={menuRef}
            side={menuSide}
            gap={TOOLTIP_GAP_PX}
            className="pointer-events-auto z-30 overflow-hidden whitespace-nowrap rounded-md border border-zinc-700/60 bg-zinc-900 text-xs text-zinc-200 shadow-lg shadow-black/40"
          >
            <DockContextMenuRow
              onClick={() => {
                setMenuOpen(false);
                onRemove?.(id);
              }}
            >
              <Trash2 className="h-3.5 w-3.5" />
              Удалить разделитель
            </DockContextMenuRow>
          </DockOverlayAnchor>
        )}

        <div
          ref={anchorRef}
          className={`flex shrink-0 ${
            isVertical
              ? "items-center justify-center"
              : "items-end justify-center"
          }`}
          style={
            isVertical
              ? { height: DOCK_SEPARATOR_WIDTH_PX, width: "100%" }
              : { width: DOCK_SEPARATOR_WIDTH_PX, height: "100%" }
          }
        >
          <DockRowDivider iconSizePx={iconSizePx} isVertical={isVertical} />
        </div>
      </div>
    </div>
  );
}
