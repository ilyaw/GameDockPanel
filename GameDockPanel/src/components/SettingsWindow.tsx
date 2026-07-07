import { useCallback, type ReactNode } from "react";
import { SlidersHorizontal } from "lucide-react";
import { useDockSettings } from "../hooks/useDockSettings";
import type { DockSettings } from "../lib/types";

/** Minimal accessible on/off switch — no toggle primitive in lucide-react,
 * and pulling in a UI library for one control isn't warranted here. */
function ToggleSwitch({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
        checked ? "bg-indigo-500" : "bg-zinc-700"
      }`}
    >
      <span
        className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
          checked ? "translate-x-5" : "translate-x-0"
        }`}
      />
    </button>
  );
}

function SettingsRow({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-6 py-4">
      <div>
        <p className="text-sm font-medium text-zinc-200">{title}</p>
        {description && (
          <p className="mt-0.5 max-w-xs text-xs text-zinc-500">{description}</p>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

const colorInputClass =
  "h-8 w-8 cursor-pointer rounded border border-zinc-700 bg-transparent p-0 disabled:cursor-not-allowed";

export function SettingsWindow() {
  const { settings, commit } = useDockSettings();

  const update = useCallback(
    (patch: Partial<DockSettings>) => {
      commit({ ...settings, ...patch });
    },
    [settings, commit],
  );

  const updateGlowColor = useCallback(
    (index: number, value: string) => {
      const next = [...settings.rgbGlowColors];
      next[index] = value;
      update({ rgbGlowColors: next });
    },
    [settings.rgbGlowColors, update],
  );

  return (
    <div className="min-h-screen w-full overflow-y-auto bg-zinc-950 px-6 py-6 text-zinc-100">
      <header className="mb-4 flex items-center gap-2">
        <SlidersHorizontal className="h-5 w-5 text-zinc-400" />
        <h1 className="text-lg font-semibold">Dock Settings</h1>
      </header>

      <section className="divide-y divide-zinc-800 rounded-xl border border-zinc-800 bg-zinc-900/60 px-4">
        <SettingsRow
          title="Animations"
          description="Cycling RGB frame and LED pulse only — hover-magnify and drag-reorder are unaffected."
        >
          <ToggleSwitch
            checked={settings.animationsEnabled}
            onChange={(value) => update({ animationsEnabled: value })}
          />
        </SettingsRow>

        <SettingsRow
          title="RGB frame colors"
          description="Cycle stops for the animated frame. Dimmed while animations are off — they're not visible until re-enabled."
        >
          <div
            className={`flex gap-2 transition-opacity ${
              settings.animationsEnabled ? "" : "opacity-40"
            }`}
          >
            {settings.rgbGlowColors.map((color, index) => (
              <input
                // Fixed 6-stop palette — index is a stable identity here.
                key={index}
                type="color"
                value={color}
                disabled={!settings.animationsEnabled}
                onChange={(event) => updateGlowColor(index, event.target.value)}
                className={colorInputClass}
                aria-label={`RGB frame color ${index + 1}`}
              />
            ))}
          </div>
        </SettingsRow>

        <SettingsRow
          title="Static frame color"
          description="Shown instead of the cycle when animations are off."
        >
          <input
            type="color"
            value={settings.staticGlowColor}
            onChange={(event) => update({ staticGlowColor: event.target.value })}
            className={colorInputClass}
            aria-label="Static frame color"
          />
        </SettingsRow>

        <SettingsRow
          title="Background tint"
          description="Color layer under the icons, painted on top of the native glass blur."
        >
          <input
            type="color"
            value={settings.tintColor}
            onChange={(event) => update({ tintColor: event.target.value })}
            className={colorInputClass}
            aria-label="Background tint color"
          />
        </SettingsRow>

        <SettingsRow
          title="Tint opacity"
          description="Alpha of the tint layer above, not the blur itself — at 100% the blur is fully hidden underneath, which is expected."
        >
          <div className="flex w-36 items-center gap-3">
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round(settings.tintOpacity * 100)}
              onChange={(event) =>
                update({ tintOpacity: Number(event.target.value) / 100 })
              }
              className="w-24 accent-indigo-500"
              aria-label="Tint opacity"
            />
            <span className="w-9 text-right text-xs tabular-nums text-zinc-400">
              {Math.round(settings.tintOpacity * 100)}%
            </span>
          </div>
        </SettingsRow>
      </section>
    </div>
  );
}
