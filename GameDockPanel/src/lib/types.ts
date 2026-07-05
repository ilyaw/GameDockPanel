/**
 * A single dock entry. This is the shared contract between the mock data
 * used today and the future Rust process-monitoring command that will
 * replace it — keep both sides of that boundary compatible with this shape.
 */
export interface DockApp {
  id: string;
  name: string;
  /** Remote/bundled icon URL. Always pair with an onError fallback — see DockIcon. */
  iconUrl: string;
  /** Whether the app is currently running. Mocked for now, toggled by click. */
  isActive: boolean;
  /**
   * Tailwind `text-*` class. Sets `currentColor`, reused by the LED both for
   * its fill (`bg-current`) and its glow (`box-shadow: currentColor` in the
   * `led-pulse` keyframes) — one field to keep both in sync.
   */
  color: string;
}
