import { useEffect, useRef } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { IS_WINDOWS } from "../lib/windowsDock";

/**
 * Windows-only fallback for macOS's global click-to-dismiss (`dock-global-mousedown`,
 * a HID-level `CGEventTap` — see `platform::macos::start_dock_click_tap`).
 * Windows deliberately has no equivalent global mouse hook (`WH_MOUSE_LL` is
 * flagged by game anti-cheats — see `platform/windows/input.rs`), so a click
 * on the desktop or another app's window never reaches this webview's own
 * listeners, and an open context menu would otherwise stay open forever.
 *
 * Window focus loss is a reliable proxy on Windows: right-clicking the dock
 * to open a menu focuses this (focusable) window, and any click elsewhere
 * (desktop, another app) takes focus back — so closing on blur reaches
 * parity without a global hook. In-window dismiss (click on another icon,
 * Escape) is unaffected — that already works via each menu's own DOM
 * listeners.
 *
 * No-op on macOS: `dock-global-mousedown` already covers the same gap
 * there, and the dock window is never focused in the first place
 * (`focus: false`, `acceptFirstMouse: true`), so focus/blur is not a
 * meaningful signal on that platform.
 */
export function useCloseMenuOnWindowBlur(menuOpen: boolean, close: () => void) {
  const closeRef = useRef(close);
  closeRef.current = close;

  useEffect(() => {
    if (!IS_WINDOWS || !menuOpen) return;

    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void getCurrentWebviewWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) {
          console.info("[menu] closing on window blur (Windows fallback)");
          closeRef.current();
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((error: unknown) => {
        console.error("[menu] onFocusChanged subscribe failed:", error);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [menuOpen]);
}
