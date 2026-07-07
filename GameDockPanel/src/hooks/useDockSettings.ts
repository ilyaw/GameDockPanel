import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { DEFAULT_DOCK_SETTINGS } from "../lib/constants";
import type { DockSettings } from "../lib/types";

/** How long to coalesce rapid `commit()` calls before persisting/broadcasting
 * — one continuous drag on the opacity slider (or a native color picker,
 * which can also fire many `onChange` events while dragging) shouldn't turn
 * into a write + IPC broadcast per intermediate value. Applied uniformly to
 * every field rather than singling out "continuous" controls: simpler, and
 * imperceptible for discrete ones (toggle, color commit) that only fire
 * once or twice anyway. */
const COMMIT_DEBOUNCE_MS = 120;

/**
 * Owns the live `DockSettings` snapshot for whichever window calls it:
 * initial pull via `get_dock_settings`, then kept in sync by
 * `dock-settings-changed` pushes (emitted by `update_dock_settings` on
 * *any* window's edit, including this one's own — harmless, just an
 * idempotent re-apply). The dock window only ever reads `settings`; the
 * settings window also calls `commit` to edit them.
 */
export function useDockSettings() {
  const [settings, setSettings] = useState<DockSettings>(DEFAULT_DOCK_SETTINGS);
  const pendingRef = useRef<DockSettings | null>(null);
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void (async () => {
      const snapshot = await invoke<DockSettings>("get_dock_settings");
      if (cancelled) return;
      setSettings(snapshot);

      unlisteners.push(
        await listen<DockSettings>("dock-settings-changed", (event) => {
          setSettings(event.payload);
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
      }
    })();

    return () => {
      cancelled = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
      clearTimeout(debounceTimer.current);
    };
  }, []);

  /**
   * Applies `next` immediately to local state (instant visual feedback in
   * the settings window itself), and debounces the actual persist +
   * cross-window broadcast. Always sends the full snapshot — simpler than
   * diffing, and the caller already has the complete object in hand.
   */
  const commit = useCallback((next: DockSettings) => {
    setSettings(next);
    pendingRef.current = next;
    clearTimeout(debounceTimer.current);
    debounceTimer.current = setTimeout(() => {
      const toSend = pendingRef.current;
      pendingRef.current = null;
      if (!toSend) return;
      invoke("update_dock_settings", { settings: toSend }).catch((error: unknown) => {
        console.error("Failed to persist dock settings:", error);
      });
    }, COMMIT_DEBOUNCE_MS);
  }, []);

  return { settings, commit };
}
