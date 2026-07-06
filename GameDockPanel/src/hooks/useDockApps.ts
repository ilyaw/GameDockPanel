import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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

/**
 * Owns the dock's app list and every IPC touchpoint around it: initial
 * snapshot, running/icon/list-membership push events, reorder, and
 * add/remove mutations (add via Finder drag-drop onto the window).
 */
export function useDockApps() {
  const [apps, setApps] = useState<DockApp[]>([]);
  const [fileDragOver, setFileDragOver] = useState(false);
  const appsRef = useRef<DockApp[]>([]);

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

      unlisteners.push(
        await getCurrentWebviewWindow().onDragDropEvent((event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            setFileDragOver(true);
          }
          if (event.payload.type === "leave" || event.payload.type === "drop") {
            setFileDragOver(false);
          }

          if (event.payload.type !== "drop") return;
          if (appsRef.current.length >= MAX_APPS) {
            console.error("dock is full");
            return;
          }

          for (const path of event.payload.paths) {
            if (!isAppBundlePath(path)) continue;
            invoke("add_app_from_path", { path }).catch((error: unknown) => {
              console.error("Failed to add app from drop:", error);
            });
            break;
          }
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
  }, []);

  const activateApp = useCallback((id: string) => {
    const app = appsRef.current.find((candidate) => candidate.id === id);
    if (!app) return;
    invoke("launch_or_activate_app", { bundleId: app.bundleId }).catch(
      (error: unknown) => {
        console.error(`Failed to launch or activate ${app.name}:`, error);
      },
    );
  }, []);

  const reorderApps = useCallback(async (newOrder: DockApp[]) => {
    const previous = appsRef.current;
    setApps(newOrder);
    try {
      await invoke("reorder_apps", {
        orderedBundleIds: newOrder.map((app) => app.bundleId),
      });
    } catch (error) {
      console.error("Failed to reorder dock:", error);
      setApps(previous);
    }
  }, []);

  const removeApp = useCallback(async (bundleId: string) => {
    try {
      await invoke("remove_app", { bundleId });
    } catch (error) {
      console.error("Failed to remove app from dock:", error);
    }
  }, []);

  return { apps, appsRef, activateApp, reorderApps, removeApp, fileDragOver };
}
