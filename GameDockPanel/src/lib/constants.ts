/**
 * Dock layout numbers used by JS logic (not styling itself — the actual
 * visuals stay in Tailwind utility classes on DockIcon/DockPanel, since
 * Tailwind's scanner needs literal class names, not interpolated values).
 * Kept here so future hover-magnify math (cursor distance -> icon scale)
 * has a single source of truth instead of re-reading pixels off the DOM.
 *
 * IMPORTANT: these must stay in sync with the Tailwind classes they
 * describe — each constant below says which class it mirrors.
 */

/** Mirrors `h-14 w-14` on the icon image/fallback badge. */
export const ICON_SIZE_PX = 56;

/** Mirrors `gap-4` between icons in the dock pill. */
export const DOCK_GAP_PX = 16;

/** Mirrors `px-5` on the dock pill. */
export const DOCK_PADDING_X_PX = 20;

/** Mirrors `py-3.5` on the dock pill. */
export const DOCK_PADDING_Y_PX = 14;

/**
 * Upper bound for how large an icon may grow during the future
 * hover-magnify pass (not implemented yet). The Tauri window
 * (src-tauri/src/platform/macos.rs) already reserves headroom for this
 * scale so magnify won't need a dynamic window resize later.
 */
export const MAGNIFY_MAX_SCALE = 1.4;
