import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

/** Mirrors Rust `WindowsBackdropSnapshot` (camelCase). */
export type WindowsDiagSnapshot = {
  lastPillClient: {
    x: number;
    y: number;
    width: number;
    height: number;
  } | null;
  menuOverlayActive: boolean;
  regionRelaxed: boolean;
  menuRegionHold: boolean;
  scaleFactor: number | null;
  innerSizePx: [number, number] | null;
  outerSizePx: [number, number] | null;
  outerPositionPx: [number, number] | null;
  storedPillDip: [number, number] | null;
  dockPosition: string;
  syncVibrancyCalls: number;
  setRgnOkCount: number;
  setRgnErrCount: number;
  gwlStyle: number | null;
  gwlExstyle: number | null;
  chromeDeltaPx: [number, number] | null;
  micaEnabled: boolean;
  hasCaption: boolean;
  isPopup: boolean;
  isLayered: boolean;
  isTransparentEx: boolean;
  chromeSubclassInstalled: boolean;
  webviewChildClass: string | null;
  chromeRepairCount: number;
  layeredRestoreCount: number;
  captionCreepCount: number;
  healthIssues: string[];
  healthy: boolean;
};

function hexStyle(n: number | null): string {
  if (n == null) return "?";
  return `0x${(n >>> 0).toString(16).padStart(8, "0")}`;
}

function pair(p: [number, number] | null | undefined, digits = 0): string {
  if (!p) return "?";
  if (digits === 0) return `${p[0]}×${p[1]}`;
  return `${p[0].toFixed(digits)}×${p[1].toFixed(digits)}`;
}

/**
 * Windows chrome HUD — magenta = HWND outer, cyan = measured CSS pill.
 * Bad health paints a red wash so screenshots show the failure mode.
 */
export function WindowsDebugHud({ enabled }: { enabled: boolean }) {
  const [snap, setSnap] = useState<WindowsDiagSnapshot | null>(null);

  useEffect(() => {
    if (!enabled) {
      setSnap(null);
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<WindowsDiagSnapshot>("dock-win-diag", (event) => {
      if (!cancelled) setSnap(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [enabled]);

  if (!enabled) return null;

  const pill = snap?.lastPillClient;
  const bad = snap != null && !snap.healthy;

  return (
    <div
      className="pointer-events-none fixed inset-0 z-[9999] overflow-hidden"
      data-win-debug-hud
      aria-hidden
    >
      {/* Outer HWND — magenta dashed */}
      <div
        className="absolute inset-0 border-2 border-dashed border-fuchsia-500/90"
        style={{ boxShadow: bad ? "inset 0 0 0 9999px rgba(220,38,38,0.18)" : undefined }}
      />

      {/* CSS pill box — cyan */}
      {pill && pill.width > 0 && pill.height > 0 && (
        <div
          className="absolute border-2 border-cyan-400/95"
          style={{
            left: pill.x,
            top: pill.y,
            width: pill.width,
            height: pill.height,
            borderRadius: 28,
          }}
        />
      )}

      {/* Status card */}
      <div
        className={`absolute left-1 top-1 max-w-[min(100%,420px)] rounded-md px-2 py-1.5 font-mono text-[10px] leading-snug text-white shadow-lg ${
          bad ? "bg-red-950/90 ring-1 ring-red-400" : "bg-black/85 ring-1 ring-fuchsia-400/60"
        }`}
      >
        {!snap ? (
          <p className="text-zinc-300">win-diag: waiting for first tick…</p>
        ) : (
          <>
            <p className={bad ? "font-semibold text-red-300" : "font-semibold text-emerald-300"}>
              {snap.healthy ? "HEALTHY" : `BAD: ${snap.healthIssues.join(", ")}`}
            </p>
            <p>
              pos={snap.dockPosition} outer={pair(snap.outerSizePx)} @
              {snap.outerPositionPx
                ? `(${snap.outerPositionPx[0]},${snap.outerPositionPx[1]})`
                : "?"}{" "}
              Δchrome={pair(snap.chromeDeltaPx)}
            </p>
            <p>
              pill={
                pill
                  ? `${pill.width.toFixed(0)}×${pill.height.toFixed(0)}@(${pill.x.toFixed(0)},${pill.y.toFixed(0)})`
                  : "?"
              }{" "}
              stored={pair(snap.storedPillDip, 0)} scale={snap.scaleFactor?.toFixed(2) ?? "?"}
            </p>
            <p>
              CAPTION={snap.hasCaption ? "1" : "0"} POPUP={snap.isPopup ? "1" : "0"} LAYERED=
              {snap.isLayered ? "1" : "0"} TRANSP={snap.isTransparentEx ? "1" : "0"} subclass=
              {snap.chromeSubclassInstalled ? "1" : "0"}
            </p>
            <p>
              STYLE={hexStyle(snap.gwlStyle)} EX={hexStyle(snap.gwlExstyle)}
            </p>
            <p>
              rgn ok/err={snap.setRgnOkCount}/{snap.setRgnErrCount} sync={snap.syncVibrancyCalls}{" "}
              relax={snap.regionRelaxed ? "1" : "0"} hold={snap.menuRegionHold ? "1" : "0"}
            </p>
            <p>
              repairs={snap.chromeRepairCount} layered↑={snap.layeredRestoreCount} caption↑=
              {snap.captionCreepCount} child={snap.webviewChildClass ?? "none"}
            </p>
            <p className="text-fuchsia-300/90">magenta=HWND · cyan=pill</p>
          </>
        )}
      </div>
    </div>
  );
}
