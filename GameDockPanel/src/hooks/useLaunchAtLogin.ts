import { useCallback, useEffect, useState } from "react";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  return String(error);
}

/**
 * Live autostart state from the OS registration managed by
 * `tauri-plugin-autostart` — not persisted in `dock-settings.json`.
 */
export function useLaunchAtLogin() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const registered = await isEnabled();
    setEnabled(registered);
    return registered;
  }, []);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const registered = await isEnabled();
        if (!cancelled) {
          setEnabled(registered);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) {
          console.error("Failed to read launch-at-login state:", err);
          setError(errorMessage(err));
          setEnabled(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  const toggle = useCallback(
    async (next: boolean) => {
      setBusy(true);
      setError(null);
      try {
        if (next) {
          await enable();
        } else {
          await disable();
        }
        await refresh();
      } catch (err) {
        console.error("Failed to update launch-at-login:", err);
        setError(errorMessage(err));
        try {
          await refresh();
        } catch (refreshErr) {
          console.error("Failed to re-sync launch-at-login state:", refreshErr);
        }
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  return { enabled, busy, error, toggle };
}
