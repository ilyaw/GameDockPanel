import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { DockOverlayAnchor } from "./DockOverlayAnchor";
import { TOOLTIP_GAP_PX } from "../lib/constants";
import { resolveOverlaySide, type OverlaySide } from "../lib/dockPlacement";
import { useCloseMenuOnWindowBlur } from "../hooks/useCloseMenuOnWindowBlur";

interface WindowLogicalPoint {
  x: number;
  y: number;
}

interface DockPillContextMenuProps {
  open: boolean;
  anchorRef: RefObject<HTMLDivElement | null>;
  overlayPreferredSide: OverlaySide;
  onClose: () => void;
  onContextMenuOpenChange?: (open: boolean) => void;
  onContextMenuBoundsChange?: (rect: DOMRect | null) => void;
  children: ReactNode;
}

/** Context menu anchored to a point on the dock pill — shared overlay plumbing. */
export function DockPillContextMenu({
  open,
  anchorRef,
  overlayPreferredSide,
  onClose,
  onContextMenuOpenChange,
  onContextMenuBoundsChange,
  children,
}: DockPillContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuSide, setMenuSide] = useState<OverlaySide>(overlayPreferredSide);

  useEffect(() => {
    onContextMenuOpenChange?.(open);
    return () => {
      if (open) onContextMenuOpenChange?.(false);
    };
  }, [open, onContextMenuOpenChange]);

  useEffect(() => {
    if (!open) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };

    const handlePointerDown = (event: MouseEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      onClose();
    };

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onClose]);

  useEffect(() => {
    if (!open) return;

    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void listen<WindowLogicalPoint>("dock-global-mousedown", (event) => {
      const rect = menuRef.current?.getBoundingClientRect();
      if (!rect) return;
      const { x, y } = event.payload;
      const insideMenu =
        x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
      if (!insideMenu) onClose();
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [open, onClose]);

  // Windows has no `dock-global-mousedown` equivalent (see hook doc) — close
  // on focus loss instead. No-op on macOS.
  useCloseMenuOnWindowBlur(open, onClose);

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

    if (!open) {
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
  }, [open, overlayPreferredSide, onContextMenuBoundsChange, anchorRef]);

  if (!open) return null;

  return (
    <DockOverlayAnchor
      innerRef={menuRef}
      side={menuSide}
      gap={TOOLTIP_GAP_PX}
      className="pointer-events-auto z-30 overflow-hidden whitespace-nowrap rounded-md border border-zinc-700/60 bg-zinc-900 text-xs text-zinc-200 shadow-lg shadow-black/40"
    >
      {children}
    </DockOverlayAnchor>
  );
}
