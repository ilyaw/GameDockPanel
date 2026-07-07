import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { AppIconUpdate, AppRunningUpdate, DockItem } from "../lib/types";
import { countDockApps, isDockAppItem } from "../lib/types";
import { MAX_APPS } from "../lib/constants";

function mergeRunningUpdates(
  items: DockItem[],
  updates: AppRunningUpdate[],
): DockItem[] {
  const byId = new Map(updates.map((update) => [update.id, update.isActive]));
  return items.map((item) => {
    if (!isDockAppItem(item)) return item;
    const isActive = byId.get(item.id);
    return isActive === undefined ? item : { ...item, isActive };
  });
}

function mergeIconUpdates(
  items: DockItem[],
  updates: AppIconUpdate[],
): DockItem[] {
  const byId = new Map(updates.map((update) => [update.id, update.iconUrl]));
  return items.map((item) => {
    if (!isDockAppItem(item)) return item;
    const iconUrl = byId.get(item.id);
    return iconUrl === undefined ? item : { ...item, iconUrl };
  });
}

function isAppBundlePath(path: string): boolean {
  return path.endsWith(".app") || path.includes(".app/");
}

export type InsertIndexResolver = (x: number, y: number) => number;

export type SeparatorPlacement = "before" | "after";

/**
 * Owns the dock's item list and every IPC touchpoint around it: initial
 * snapshot, running/icon/list-membership push events, reorder, and
 * add/remove mutations (add via Finder drag-drop onto the window — the
 * only add path — funnels through `addAppPath` below into the
 * `add_app_from_path` command).
 */
export function useDockApps() {
  const [items, setItems] = useState<DockItem[]>([]);
  const [fileDragOver, setFileDragOver] = useState(false);
  const [fileDragInsertIndex, setFileDragInsertIndex] = useState<number | null>(
    null,
  );
  const [rejectPulseKey, setRejectPulseKey] = useState(0);
  const itemsRef = useRef<DockItem[]>([]);
  const resolveInsertIndexRef = useRef<InsertIndexResolver | null>(null);
  const dragScaleFactorRef = useRef(1);

  const reportReject = useCallback(() => {
    setRejectPulseKey((key) => key + 1);
  }, []);

  const resolveInsertIndexFromPosition = useCallback(
    (position: PhysicalPosition): number => {
      const logical = position.toLogical(dragScaleFactorRef.current);
      return (
        resolveInsertIndexRef.current?.(logical.x, logical.y) ??
        itemsRef.current.length
      );
    },
    [],
  );

  const updateFileDragInsertIndex = useCallback(
    (position: PhysicalPosition) => {
      const index = resolveInsertIndexFromPosition(position);
      setFileDragInsertIndex(index);
    },
    [resolveInsertIndexFromPosition],
  );

  const addAppPath = useCallback(
    async (path: string, insertIndex?: number) => {
      try {
        await invoke("add_app_from_path", {
          path,
          insertIndex: insertIndex ?? null,
        });
      } catch (error) {
        reportReject();
        throw error;
      }
    },
    [reportReject],
  );

  useEffect(() => {
    itemsRef.current = items;
  }, [items]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void (async () => {
      const snapshot = await invoke<DockItem[]>("get_apps_snapshot");
      if (cancelled) return;
      setItems(snapshot);

      unlisteners.push(
        await listen<AppRunningUpdate[]>("apps-running-changed", (event) => {
          setItems((prev) => mergeRunningUpdates(prev, event.payload));
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
        return;
      }

      unlisteners.push(
        await listen<AppIconUpdate[]>("apps-icons-updated", (event) => {
          setItems((prev) => mergeIconUpdates(prev, event.payload));
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
        return;
      }

      unlisteners.push(
        await listen<DockItem[]>("apps-list-changed", (event) => {
          setItems(event.payload);
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
        return;
      }

      const webview = getCurrentWebviewWindow();

      unlisteners.push(
        await webview.onDragDropEvent((event) => {
          if (event.payload.type === "enter") {
            void webview.scaleFactor().then((scaleFactor) => {
              dragScaleFactorRef.current = scaleFactor;
            });
            setFileDragOver(true);
            updateFileDragInsertIndex(event.payload.position);
            return;
          }

          if (event.payload.type === "over") {
            setFileDragOver(true);
            updateFileDragInsertIndex(event.payload.position);
            return;
          }

          if (event.payload.type === "leave") {
            setFileDragOver(false);
            setFileDragInsertIndex(null);
            return;
          }

          if (event.payload.type !== "drop") return;

          setFileDragOver(false);
          const insertIndex = resolveInsertIndexFromPosition(event.payload.position);
          setFileDragInsertIndex(null);

          if (countDockApps(itemsRef.current) >= MAX_APPS) {
            console.error("dock is full");
            reportReject();
            return;
          }

          const appPath = event.payload.paths.find(isAppBundlePath);
          if (!appPath) {
            reportReject();
            return;
          }

          addAppPath(appPath, insertIndex).catch((error: unknown) => {
            console.error("Failed to add app from drop:", error);
          });
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
    };
  }, [addAppPath, reportReject, resolveInsertIndexFromPosition, updateFileDragInsertIndex]);

  const activateApp = useCallback((id: string) => {
    const app = itemsRef.current.find(
      (candidate): candidate is Extract<DockItem, { type: "app" }> =>
        isDockAppItem(candidate) && candidate.id === id,
    );
    if (!app) return;
    invoke("launch_or_activate_app", { bundleId: app.bundleId }).catch(
      (error: unknown) => {
        console.error(`Failed to launch or activate ${app.name}:`, error);
      },
    );
  }, []);

  const reorderItems = useCallback((newOrder: DockItem[]) => {
    setItems(newOrder);
  }, []);

  const commitReorder = useCallback(async () => {
    try {
      await invoke("reorder_apps", {
        orderedIds: itemsRef.current.map((item) => item.id),
      });
    } catch (error) {
      console.error("Failed to persist dock reorder:", error);
      try {
        const snapshot = await invoke<DockItem[]>("get_apps_snapshot");
        setItems(snapshot);
      } catch (resyncError) {
        console.error("Failed to resync dock after reorder failure:", resyncError);
      }
    }
  }, []);

  const removeApp = useCallback(async (bundleId: string) => {
    try {
      await invoke("remove_app", { bundleId });
    } catch (error) {
      console.error("Failed to remove app from dock:", error);
    }
  }, []);

  const insertSeparator = useCallback(
    async (bundleId: string, placement: SeparatorPlacement) => {
      try {
        await invoke("insert_separator", {
          bundleId,
          placement,
          separatorId: crypto.randomUUID(),
        });
      } catch (error) {
        console.error("Failed to insert separator:", error);
        reportReject();
      }
    },
    [reportReject],
  );

  const removeSeparator = useCallback(async (separatorId: string) => {
    try {
      await invoke("remove_separator", { separatorId });
    } catch (error) {
      console.error("Failed to remove separator:", error);
    }
  }, []);

  const showInFinder = useCallback((bundleId: string) => {
    invoke("reveal_app_in_finder", { bundleId }).catch((error: unknown) => {
      console.error(`Failed to reveal ${bundleId} in Finder:`, error);
    });
  }, []);

  const quitApp = useCallback((bundleId: string) => {
    invoke("quit_app", { bundleId }).catch((error: unknown) => {
      console.error(`Failed to quit ${bundleId}:`, error);
    });
  }, []);

  const setIndicatorColor = useCallback(
    async (bundleId: string, color: string | null) => {
      setItems((prev) =>
        prev.map((item) => {
          if (!isDockAppItem(item) || item.bundleId !== bundleId) return item;
          return {
            ...item,
            indicatorColorOverride: color,
            indicatorColor: color ?? item.indicatorColorAuto,
          };
        }),
      );

      try {
        await invoke("set_app_indicator_color", { bundleId, color });
      } catch (error) {
        console.error(`Failed to set indicator color for ${bundleId}:`, error);
        const snapshot = await invoke<DockItem[]>("get_apps_snapshot");
        setItems(snapshot);
      }
    },
    [],
  );

  return {
    items,
    itemsRef,
    activateApp,
    reorderItems,
    commitReorder,
    removeApp,
    insertSeparator,
    removeSeparator,
    fileDragOver,
    fileDragInsertIndex,
    resolveInsertIndexRef,
    rejectPulseKey,
    showInFinder,
    quitApp,
    setIndicatorColor,
  };
}
