import React from "react";
import ReactDOM from "react-dom/client";
import { attachConsole } from "@tauri-apps/plugin-log";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import "./index.css";

void attachConsole();

function reportFrontendError(
  message: string,
  source?: string,
  line?: number,
): void {
  void invoke("log_frontend_error", {
    message,
    source: source ?? null,
    line: line ?? null,
  }).catch(() => undefined);
}

window.onerror = (event, source, lineno, _colno, error) => {
  const message =
    error instanceof Error
      ? `${error.name}: ${error.message}`
      : typeof event === "string"
        ? event
        : "window.onerror";
  reportFrontendError(message, source ?? undefined, lineno ?? undefined);
  return false;
};

window.onunhandledrejection = (event: PromiseRejectionEvent) => {
  const reason = event.reason;
  const message =
    reason instanceof Error
      ? `${reason.name}: ${reason.message}`
      : `unhandledrejection: ${String(reason)}`;
  reportFrontendError(message, "unhandledrejection");
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
