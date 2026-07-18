import { invoke } from "@tauri-apps/api/core";

/** WebView2 / Windows dock — used to gate hover/menu HWND expand calls. */
export const IS_WINDOWS =
  typeof navigator !== "undefined" &&
  navigator.platform.toLowerCase().includes("win");

export type DockRegionRelaxOptions = {
  /**
   * When opening a context menu, pass `true` so the click-through poller
   * cannot shrink the HWND before `set_menu_overlay` lands. Pass
   * `false` when the menu fully closes.
   */
  menuHold?: boolean;
};

/**
 * Windows only: expand (`true`) or shrink the dock HWND for hover / menu.
 * Await before opening a context menu so the first paint has room.
 */
export async function setDockRegionRelaxed(
  relaxed: boolean,
  options?: DockRegionRelaxOptions,
): Promise<void> {
  if (!IS_WINDOWS) return;
  try {
    await invoke("set_dock_region_relaxed", {
      relaxed,
      menuHold: options?.menuHold ?? null,
    });
  } catch (error: unknown) {
    console.error("[dock] set_dock_region_relaxed failed:", error);
  }
}
