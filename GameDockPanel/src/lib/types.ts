/**
 * A single dock entry. This is the shared contract between the frontend and
 * the Rust process-monitoring commands (`get_apps_snapshot` /
 * `apps-state-changed`) that own this data — keep both sides of that
 * boundary compatible with this shape.
 */
export interface DockApp {
  id: string;
  name: string;
  /** macOS bundle identifier — sent back to `launch_or_activate_app` on click. */
  bundleId: string;
  /**
   * Native icon rendered by Rust as a `data:image/png;base64,...` URL, or
   * `null` if it couldn't be resolved (app not installed). Always pair with
   * a fallback — see DockIcon.
   */
  iconUrl: string | null;
  /** Whether the app is currently running, per `NSWorkspace` — not mocked. */
  isActive: boolean;
  /**
   * Tailwind `text-*` class. Sets `currentColor`, reused by the LED both for
   * its fill (`bg-current`) and its glow (`box-shadow: currentColor` in the
   * `led-pulse` keyframes) — one field to keep both in sync.
   */
  color: string;
}
