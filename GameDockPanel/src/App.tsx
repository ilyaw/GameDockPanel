import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { DockPanel } from "./components/DockPanel";
import { SettingsWindow } from "./components/SettingsWindow";

/**
 * Both the dock and the settings window load this exact same bundle
 * (`WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html"...))`
 * in `commands/window.rs`) — which UI to render is decided by the current
 * window's own label, a plain synchronous property with no extra IPC round
 * trip. Deliberately not a query/hash-based route: there's no router in
 * this project, and adding one for a single static branch would be more
 * machinery than the problem needs.
 */
function App() {
  const label = getCurrentWebviewWindow().label;
  return label === "settings" ? (
    <SettingsWindow />
  ) : (
    <div className="dock-root h-full">
      <DockPanel />
    </div>
  );
}

export default App;
