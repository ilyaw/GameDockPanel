import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { AppIconUpdate, AppRunningUpdate, DockApp } from "../lib/types";
import { MAX_APPS } from "../lib/constants";

function mergeRunningUpdates(
  apps: DockApp[],
  updates: AppRunningUpdate[],
): DockApp[] {
  const byId = new Map(updates.map((update) => [update.id, update.isActive]));
  return apps.map((app) => {
    const isActive = byId.get(app.id);
    return isActive === undefined ? app : { ...app, isActive };
  });
}

function mergeIconUpdates(apps: DockApp[], updates: AppIconUpdate[]): DockApp[] {
  const byId = new Map(updates.map((update) => [update.id, update.iconUrl]));
  return apps.map((app) => {
    const iconUrl = byId.get(app.id);
    return iconUrl === undefined ? app : { ...app, iconUrl };
  });
}

function isAppBundlePath(path: string): boolean {
  return path.endsWith(".app") || path.includes(".app/");
}

export type InsertIndexResolver = (x: number, y: number) => number;

/**
 * Owns the dock's app list and every IPC touchpoint around it: initial
 * snapshot, running/icon/list-membership push events, reorder, and
 * add/remove mutations (add via Finder drag-drop onto the window — the
 * only add path — funnels through `addAppPath` below into the
 * `add_app_from_path` command).
 */
export function useDockApps() {
  const [apps, setApps] = useState<DockApp[]>([]);
  const [fileDragOver, setFileDragOver] = useState(false);
  const [fileDragInsertIndex, setFileDragInsertIndex] = useState<number | null>(
    null,
  );
  /**
   * Bumped on any add-app rejection — invalid file type on drop, duplicate
   * bundle ID, or a full dock. One counter, not a boolean, so `DockPanel`
   * can re-trigger the reject animation on consecutive rejections via a
   * `key`/effect dependency even if the value would otherwise "already be
   * true".
   */
  const [rejectPulseKey, setRejectPulseKey] = useState(0);
  const appsRef = useRef<DockApp[]>([]);
  /** Filled by `DockPanel` — maps drag cursor coords to a dock insert slot. */
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
        appsRef.current.length
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

  /**
   * Single entry point into `add_app_from_path` — same duplicate/`MAX_APPS`
   * rejection on the Rust side, and the same reject-pulse cue on the
   * frontend either way (see `rejectPulseKey`).
   */
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
    appsRef.current = apps;
  }, [apps]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void (async () => {
      const snapshot = await invoke<DockApp[]>("get_apps_snapshot");
      if (cancelled) return;
      setApps(snapshot);

      unlisteners.push(
        await listen<AppRunningUpdate[]>("apps-running-changed", (event) => {
          setApps((prev) => mergeRunningUpdates(prev, event.payload));
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
          setApps((prev) => mergeIconUpdates(prev, event.payload));
        }),
      );

      if (cancelled) {
        for (const unlisten of unlisteners) {
          unlisten();
        }
        return;
      }

      unlisteners.push(
        await listen<DockApp[]>("apps-list-changed", (event) => {
          setApps(event.payload);
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

          if (appsRef.current.length >= MAX_APPS) {
            console.error("dock is full");
            reportReject();
            return;
          }

          const appPath = event.payload.paths.find(isAppBundlePath);
          if (!appPath) {
            // Dropped something that isn't a `.app` bundle — silence would
            // read as "nothing happened", so flash the same reject cue used
            // for duplicate/full-dock errors below.
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
    const app = appsRef.current.find((candidate) => candidate.id === id);
    if (!app) return;
    invoke("launch_or_activate_app", { bundleId: app.bundleId }).catch(
      (error: unknown) => {
        console.error(`Failed to launch or activate ${app.name}:`, error);
      },
    );
  }, []);

  /**
   * Local-only reorder — Framer Motion's `Reorder.Group.onReorder` fires this
   * once per neighbor the dragged icon crosses, i.e. potentially many times
   * during a single drag gesture, not just once at drop. Only updating React
   * state here (no IPC) keeps the on-screen reorder smooth without turning
   * every intermediate swap into a disk write; see `commitReorder` below for
   * the actual persistence, which runs exactly once per drag.
   */
  const reorderApps = useCallback((newOrder: DockApp[]) => {
    setApps(newOrder);
  }, []);

  /**
   * Persists the current order — called once from `DockPanel`'s drag-end
   * handler, never from `onReorder` directly (see `reorderApps` above; also
   * avoids N concurrent `invoke` calls whose completion order isn't
   * guaranteed to match the drag's swap order). On failure, re-fetches the
   * canonical snapshot rather than trying to reconstruct "the order before
   * the last of N swaps" — simpler and always correct.
   */
  const commitReorder = useCallback(async () => {
    try {
      await invoke("reorder_apps", {
        orderedBundleIds: appsRef.current.map((app) => app.bundleId),
      });
    } catch (error) {
      console.error("Failed to persist dock reorder:", error);
      try {
        const snapshot = await invoke<DockApp[]>("get_apps_snapshot");
        setApps(snapshot);
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

  const showInFinder = useCallback((bundleId: string) => {
    invoke("reveal_app_in_finder", { bundleId }).catch((error: unknown) => {
      console.error(`Failed to reveal ${bundleId} in Finder:`, error);
    });
  }, []);

  /**
   * Never mutates `apps`/`isActive` optimistically — the real LED flip comes
   * from the same `apps-running-changed` push that a manual Cmd+Q would
   * also trigger (see `platform::quit_app`).
   */
  const quitApp = useCallback((bundleId: string) => {
    invoke("quit_app", { bundleId }).catch((error: unknown) => {
      console.error(`Failed to quit ${bundleId}:`, error);
    });
  }, []);

  const setIndicatorColor = useCallback(
    async (bundleId: string, color: string | null) => {
      setApps((prev) =>
        prev.map((app) => {
          if (app.bundleId !== bundleId) return app;
          return {
            ...app,
            indicatorColorOverride: color,
            indicatorColor: color ?? app.indicatorColorAuto,
          };
        }),
      );

      try {
        await invoke("set_app_indicator_color", { bundleId, color });
      } catch (error) {
        console.error(`Failed to set indicator color for ${bundleId}:`, error);
        const snapshot = await invoke<DockApp[]>("get_apps_snapshot");
        setApps(snapshot);
      }
    },
    [],
  );

  return {
    apps,
    appsRef,
    activateApp,
    reorderApps,
    commitReorder,
    removeApp,
    fileDragOver,
    fileDragInsertIndex,
    resolveInsertIndexRef,
    rejectPulseKey,
    showInFinder,
    quitApp,
    setIndicatorColor,
  };
}
