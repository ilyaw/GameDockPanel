import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IS_WINDOWS } from "../lib/windowsDock";

/**
 * Hidden-start gate: show the native dock window only after React has painted
 * the pill and the first geometry sync has aligned SetWindowRgn.
 *
 * Retries while `geometryReady` stays true and show has not succeeded yet
 * (covers StrictMode cancel mid-rAF and transient invoke failures).
 */
export function useDockWindowReady(geometryReady: boolean) {
  const shownRef = useRef(false);
  const attemptRef = useRef(0);

  useEffect(() => {
    if (shownRef.current) return;
    if (!geometryReady && !IS_WINDOWS) return;

    let cancelled = false;
    const safetyMs = IS_WINDOWS ? 5000 : 0;
    let safetyTimer: number | undefined;
    let retryTimer: number | undefined;

    const show = async (reason: string) => {
      if (cancelled || shownRef.current) return;
      const attempt = ++attemptRef.current;
      try {
        if (document.fonts?.ready) {
          await document.fonts.ready.catch(() => undefined);
        }
        await new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => resolve());
          });
        });
        if (cancelled || shownRef.current) return;
        // Rust show gate waits for in-flight show; Ok means window is visible.
        await invoke("show_main_window");
        if (attempt !== attemptRef.current) return;
        shownRef.current = true;
        console.info(`[dock] show_main_window ok (${reason})`);
      } catch (error: unknown) {
        if (cancelled || shownRef.current) return;
        console.error("[dock] show_main_window failed:", error);
        void invoke("log_frontend_error", {
          message: `show_main_window failed: ${String(error)}`,
          source: "useDockWindowReady",
          line: null,
        }).catch(() => undefined);
        // Retry while still mounted and not shown (geometry may already be ready).
        if (!cancelled && !shownRef.current) {
          retryTimer = window.setTimeout(() => {
            void show("retry");
          }, 750);
        }
      }
    };

    if (geometryReady) {
      void show("geometry-ready");
    } else if (safetyMs > 0) {
      safetyTimer = window.setTimeout(() => {
        void show("safety-timeout");
      }, safetyMs);
    }

    return () => {
      cancelled = true;
      if (safetyTimer !== undefined) window.clearTimeout(safetyTimer);
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
    };
  }, [geometryReady]);
}
