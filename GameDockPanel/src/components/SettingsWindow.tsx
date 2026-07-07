import { useCallback, type ReactNode } from "react";
import { SlidersHorizontal } from "lucide-react";
import { useDockSettings } from "../hooks/useDockSettings";
import { BACKGROUND_PRESETS, type BackgroundPreset } from "../lib/constants";
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

/** Shared 0..1-as-percent slider — used by the three background animation
 * controls below (intensity/visibility/speed), which all share the same
 * "range input + right-aligned percentage readout" shape. */
function PercentSlider({
  value,
  onChange,
  disabled,
  ariaLabel,
}: {
  value: number;
  onChange: (next: number) => void;
  disabled?: boolean;
  ariaLabel: string;
}) {
  return (
    <div className="flex w-36 items-center gap-3">
      <input
        type="range"
        min={0}
        max={100}
        value={Math.round(value * 100)}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value) / 100)}
        className="w-24 accent-indigo-500 disabled:cursor-not-allowed disabled:opacity-50"
        aria-label={ariaLabel}
      />
      <span className="w-9 text-right text-xs tabular-nums text-zinc-400">
        {Math.round(value * 100)}%
      </span>
    </div>
  );
}

/** A single preset swatch — its own mini gradient preview stands in for a
 * label, so picking one is a straight visual choice instead of a name in a
 * dropdown. `aria-pressed`, not `aria-checked`: this is a set of toggle
 * buttons picking one of many, not a single on/off switch. */
function PresetSwatch({
  preset,
  selected,
  disabled,
  onSelect,
}: {
  preset: BackgroundPreset;
  selected: boolean;
  disabled?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      title={preset.label}
      aria-label={preset.label}
      aria-pressed={selected}
      disabled={disabled}
      onClick={onSelect}
      style={{ backgroundImage: `linear-gradient(135deg, ${preset.colors.join(", ")})` }}
      className={`h-8 w-8 shrink-0 rounded-lg border-2 transition-transform disabled:cursor-not-allowed disabled:opacity-40 ${
        selected ? "scale-105 border-white" : "border-transparent hover:scale-105"
      }`}
    />
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
    <div className="h-screen w-full overflow-y-auto bg-zinc-950 px-6 py-6 text-zinc-100">
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
          title="Background animation"
          description="Flowing RGB/gradient layer under the icons, painted on top of the native glass blur."
        >
          <ToggleSwitch
            checked={settings.backgroundAnimationEnabled}
            onChange={(value) => update({ backgroundAnimationEnabled: value })}
          />
        </SettingsRow>

        <SettingsRow
          title="Preset"
          description="Ready-made color combos for the flow — pick a look instead of individual colors."
        >
          <div
            className={`flex max-w-[200px] flex-wrap justify-end gap-2 transition-opacity ${
              settings.backgroundAnimationEnabled ? "" : "opacity-40"
            }`}
          >
            {BACKGROUND_PRESETS.map((preset) => (
              <PresetSwatch
                key={preset.id}
                preset={preset}
                selected={settings.backgroundPreset === preset.id}
                disabled={!settings.backgroundAnimationEnabled}
                onSelect={() => update({ backgroundPreset: preset.id })}
              />
            ))}
          </div>
        </SettingsRow>

        <SettingsRow
          title="Intensity"
          description="How vivid the flow's colors are — low mixes them toward black, high is full brightness."
        >
          <PercentSlider
            value={settings.backgroundIntensity}
            onChange={(value) => update({ backgroundIntensity: value })}
            disabled={!settings.backgroundAnimationEnabled}
            ariaLabel="Background intensity"
          />
        </SettingsRow>

        <SettingsRow
          title="Visibility"
          description="Opacity of the whole flow layer over the glass — low keeps it a subtle wash."
        >
          <PercentSlider
            value={settings.backgroundVisibility}
            onChange={(value) => update({ backgroundVisibility: value })}
            disabled={!settings.backgroundAnimationEnabled}
            ariaLabel="Background visibility"
          />
        </SettingsRow>

        <SettingsRow
          title="Speed"
          description="How fast the gradient flows — low is a slow drift, high cycles quickly."
        >
          <PercentSlider
            value={settings.backgroundSpeed}
            onChange={(value) => update({ backgroundSpeed: value })}
            disabled={!settings.backgroundAnimationEnabled}
            ariaLabel="Background flow speed"
          />
        </SettingsRow>
      </section>
    </div>
  );
}
