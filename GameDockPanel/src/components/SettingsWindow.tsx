import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { SlidersHorizontal } from "lucide-react";
import { useDockSettings } from "../hooks/useDockSettings";
import {
  BACKGROUND_PRESETS,
  BORDER_STYLE_PRESETS,
  DOCK_POSITION_OPTIONS,
  ICON_SIZE_PRESETS,
  ICON_SIZE_MAX_PX,
  ICON_SIZE_MIN_PX,
  LED_COLOR_MODE_OPTIONS,
  PANEL_EFFECT_PRESETS,
  clampIconSizePx,
  type BackgroundPreset,
  type BorderStylePreset,
  type IconSizePreset,
  type LedColorMode,
  type PanelEffectPreset,
} from "../lib/constants";
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
      title={`${preset.label} (${preset.animation})`}
      aria-label={`${preset.label}, анимация ${preset.animation}`}
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

/** Named text pill for a small, description-bearing preset list (border
 * style / panel effect) — unlike `PresetSwatch`, these presets aren't
 * colors, so a label reads better than a swatch here. */
function StylePresetButton({
  preset,
  selected,
  disabled,
  onSelect,
}: {
  preset: BorderStylePreset | PanelEffectPreset | IconSizePreset;
  selected: boolean;
  disabled?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      title={preset.description}
      aria-pressed={selected}
      disabled={disabled}
      onClick={onSelect}
      className={`rounded-lg border px-2.5 py-1 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
        selected
          ? "border-indigo-400 bg-indigo-500/20 text-indigo-200"
          : "border-zinc-700 bg-zinc-800/60 text-zinc-400 hover:border-zinc-600 hover:text-zinc-200"
      }`}
    >
      {preset.label}
    </button>
  );
}

function SettingsRow({
  title,
  description,
  descriptionClassName = "max-w-xs",
  children,
}: {
  title: string;
  description?: string;
  descriptionClassName?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-6 py-4">
      <div>
        <p className="text-sm font-medium text-zinc-200">{title}</p>
        {description && (
          <p className={`mt-0.5 text-xs text-zinc-500 ${descriptionClassName}`}>
            {description}
          </p>
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
  const [sliderIconSizePx, setSliderIconSizePx] = useState(settings.iconSizePx);
  const previewPendingPxRef = useRef<number | null>(null);
  const previewRafRef = useRef(0);
  const persistSizeTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    setSliderIconSizePx(settings.iconSizePx);
  }, [settings.iconSizePx]);

  useEffect(
    () => () => {
      if (previewRafRef.current) cancelAnimationFrame(previewRafRef.current);
      clearTimeout(persistSizeTimerRef.current);
    },
    [],
  );

  const update = useCallback(
    (patch: Partial<DockSettings>) => {
      commit({ ...settings, ...patch });
    },
    [settings, commit],
  );

  const queueIconSizePreview = useCallback((px: number) => {
    previewPendingPxRef.current = clampIconSizePx(px);
    if (previewRafRef.current) return;
    previewRafRef.current = requestAnimationFrame(() => {
      previewRafRef.current = 0;
      const pending = previewPendingPxRef.current;
      if (pending === null) return;
      previewPendingPxRef.current = null;
      void emit("dock-icon-size-preview", pending);
    });
  }, []);

  const persistIconSize = useCallback(
    (px: number, presetId?: string) => {
      const patch: Partial<DockSettings> = { iconSizePx: px };
      if (presetId) patch.iconSizePreset = presetId;
      update(patch);
    },
    [update],
  );

  const handleIconSizeSliderChange = useCallback(
    (px: number) => {
      setSliderIconSizePx(px);
      queueIconSizePreview(px);
      clearTimeout(persistSizeTimerRef.current);
      persistSizeTimerRef.current = setTimeout(() => {
        persistIconSize(px);
      }, 200);
    },
    [queueIconSizePreview, persistIconSize],
  );

  const handleIconSizeSliderRelease = useCallback(() => {
    clearTimeout(persistSizeTimerRef.current);
    persistIconSize(sliderIconSizePx);
  }, [persistIconSize, sliderIconSizePx]);

  const updateGlowColor = useCallback(
    (index: number, value: string) => {
      const next = [...settings.rgbGlowColors];
      next[index] = value;
      update({ rgbGlowColors: next });
    },
    [settings.rgbGlowColors, update],
  );

  const applyGlowPalette = useCallback(
    (preset: BackgroundPreset) => {
      update({ rgbGlowColors: [...preset.colors], staticGlowColor: preset.colors[0] });
    },
    [update],
  );

  return (
    <div className="h-screen w-full overflow-y-auto bg-zinc-950 px-6 py-6 text-zinc-100">
      <header className="mb-4 flex items-center gap-2">
        <SlidersHorizontal className="h-5 w-5 text-zinc-400" />
        <h1 className="text-lg font-semibold">Настройки дока</h1>
      </header>

      <section className="divide-y divide-zinc-800 rounded-xl border border-zinc-800 bg-zinc-900/60 px-4">
        <SettingsRow
          title="Положение панели"
          description="К какому краю экрана прикреплена панель дока."
        >
          <div className="flex max-w-[240px] flex-wrap justify-end gap-1.5">
            {DOCK_POSITION_OPTIONS.map((option) => (
              <StylePresetButton
                key={option.id}
                preset={{
                  id: option.id,
                  label: option.label,
                  description: option.description,
                }}
                selected={settings.dockPosition === option.id}
                onSelect={() => update({ dockPosition: option.id })}
              />
            ))}
          </div>
        </SettingsRow>

        <SettingsRow
          title="Размер иконок"
          descriptionClassName="max-w-sm"
          description="Ползунок и пресеты меняют размер иконок, отступы и высоту панели согласованно; окно остаётся по центру экрана."
        >
          <div className="flex w-full max-w-[280px] flex-col items-end gap-3">
            <div className="w-full">
              <div className="mb-1.5 flex justify-between text-[10px] text-zinc-500">
                <span>Компакт</span>
                <span>Крупный</span>
              </div>
              <div className="flex items-center gap-3">
                <input
                  type="range"
                  min={ICON_SIZE_MIN_PX}
                  max={ICON_SIZE_MAX_PX}
                  step={1}
                  value={sliderIconSizePx}
                  onChange={(event) =>
                    handleIconSizeSliderChange(
                      clampIconSizePx(Number(event.target.value)),
                    )
                  }
                  onPointerUp={handleIconSizeSliderRelease}
                  onKeyUp={handleIconSizeSliderRelease}
                  className="w-full accent-indigo-500"
                  aria-label="Размер иконок"
                />
                <span className="w-10 shrink-0 text-right text-xs tabular-nums text-zinc-400">
                  {sliderIconSizePx}px
                </span>
              </div>
            </div>
            <div className="flex flex-wrap justify-end gap-1.5">
              {ICON_SIZE_PRESETS.map((preset) => (
                <StylePresetButton
                  key={preset.id}
                  preset={preset}
                  selected={sliderIconSizePx === preset.iconSizePx}
                  onSelect={() => {
                    setSliderIconSizePx(preset.iconSizePx);
                    queueIconSizePreview(preset.iconSizePx);
                    clearTimeout(persistSizeTimerRef.current);
                    persistIconSize(preset.iconSizePx, preset.id);
                  }}
                />
              ))}
            </div>
          </div>
        </SettingsRow>

        <SettingsRow
          title="Анимации рамки"
          description="Переливающаяся RGB-рамка и пульсация LED — увеличение иконок при наведении и перетаскивание не затрагиваются."
        >
          <ToggleSwitch
            checked={settings.animationsEnabled}
            onChange={(value) => update({ animationsEnabled: value })}
          />
        </SettingsRow>

        <SettingsRow
          title="Индикаторы запущенных приложений"
          description="Цвет полоски под иконкой: автоматически из иконки, один для всех, или только вручную заданные."
        >
          <div className="flex max-w-[240px] flex-col items-end gap-2">
            <div className="flex flex-wrap justify-end gap-1.5">
              {LED_COLOR_MODE_OPTIONS.map((option) => (
                <StylePresetButton
                  key={option.id}
                  preset={{ id: option.id, label: option.label, description: option.description }}
                  selected={settings.ledColorMode === option.id}
                  onSelect={() => update({ ledColorMode: option.id as LedColorMode })}
                />
              ))}
            </div>
            {settings.ledColorMode === "fixed" && (
              <input
                type="color"
                value={settings.ledFixedColor}
                onChange={(event) => update({ ledFixedColor: event.target.value })}
                className={colorInputClass}
                aria-label="Фиксированный цвет индикатора"
              />
            )}
            <button
              type="button"
              onClick={() => {
                void invoke("refresh_indicator_colors");
              }}
              className="text-xs text-zinc-400 underline-offset-2 hover:text-zinc-200 hover:underline"
            >
              Пересчитать цвета из иконок
            </button>
          </div>
        </SettingsRow>

        <SettingsRow
          title="Стиль рамки"
          description="Как анимируется RGB-рамка: классические спектр/глитч/скан или spotlight-потоки (вращение, sweep, неон-пульс)."
        >
          <div
            className={`flex max-w-[280px] flex-wrap justify-end gap-1.5 transition-opacity ${
              settings.animationsEnabled ? "" : "opacity-40"
            }`}
          >
            {BORDER_STYLE_PRESETS.map((preset) => (
              <StylePresetButton
                key={preset.id}
                preset={preset}
                selected={settings.borderStyle === preset.id}
                disabled={!settings.animationsEnabled}
                onSelect={() => update({ borderStyle: preset.id })}
              />
            ))}
          </div>
        </SettingsRow>

        <SettingsRow
          title="Цвета RGB-рамки"
          description="Опорные цвета переливающейся рамки. Затемнено, пока анимации выключены — они не видны, пока анимации не включены снова."
        >
          <div className="flex flex-col items-end gap-2">
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
                  aria-label={`Цвет рамки ${index + 1}`}
                />
              ))}
            </div>
            <div
              className={`flex max-w-[200px] flex-wrap justify-end gap-1.5 transition-opacity ${
                settings.animationsEnabled ? "" : "opacity-40"
              }`}
            >
              {BACKGROUND_PRESETS.map((preset) => (
                <PresetSwatch
                  key={preset.id}
                  preset={preset}
                  selected={
                    settings.animationsEnabled &&
                    settings.rgbGlowColors.every((color, i) => color === preset.colors[i])
                  }
                  disabled={!settings.animationsEnabled}
                  onSelect={() => applyGlowPalette(preset)}
                />
              ))}
            </div>
          </div>
        </SettingsRow>

        <SettingsRow
          title="Статичный цвет рамки"
          description="Отображается вместо переливания, когда анимации выключены."
        >
          <input
            type="color"
            value={settings.staticGlowColor}
            onChange={(event) => update({ staticGlowColor: event.target.value })}
            className={colorInputClass}
            aria-label="Статичный цвет рамки"
          />
        </SettingsRow>

        <SettingsRow
          title="Эффект панели"
          description="Дополнительный киберпанк-слой поверх панели: горизонтальные ЭЛТ-линии, HUD-сетка или мерцание голограммы."
        >
          <div className="flex flex-col items-end gap-2">
            <ToggleSwitch
              checked={settings.panelEffectEnabled}
              onChange={(value) => update({ panelEffectEnabled: value })}
            />
            <div
              className={`flex max-w-[220px] flex-wrap justify-end gap-1.5 transition-opacity ${
                settings.panelEffectEnabled ? "" : "opacity-40"
              }`}
            >
              {PANEL_EFFECT_PRESETS.map((preset) => (
                <StylePresetButton
                  key={preset.id}
                  preset={preset}
                  selected={settings.panelEffect === preset.id}
                  disabled={!settings.panelEffectEnabled}
                  onSelect={() => update({ panelEffect: preset.id })}
                />
              ))}
            </div>
          </div>
        </SettingsRow>

        <SettingsRow
          title="Анимация фона"
          description="Текущий RGB/градиентный слой под иконками, отрисованный поверх нативного стеклянного блюра."
        >
          <ToggleSwitch
            checked={settings.backgroundAnimationEnabled}
            onChange={(value) => update({ backgroundAnimationEnabled: value })}
          />
        </SettingsRow>

        <SettingsRow
          title="Пресет фона"
          description="Готовые цветовые комбинации и тип анимации (sweep, spin, pulse) — каждый пресет задаёт свой движок потока."
        >
          <div
            className={`flex max-w-[240px] flex-wrap justify-end gap-2 transition-opacity ${
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
          title="Интенсивность"
          description="Насколько насыщены цвета потока — низкое значение подмешивает чёрный, высокое — полная яркость."
        >
          <PercentSlider
            value={settings.backgroundIntensity}
            onChange={(value) => update({ backgroundIntensity: value })}
            disabled={!settings.backgroundAnimationEnabled}
            ariaLabel="Интенсивность фона"
          />
        </SettingsRow>

        <SettingsRow
          title="Видимость"
          description="Непрозрачность всего слоя потока поверх стекла — низкое значение оставляет лишь лёгкий оттенок."
        >
          <PercentSlider
            value={settings.backgroundVisibility}
            onChange={(value) => update({ backgroundVisibility: value })}
            disabled={!settings.backgroundAnimationEnabled}
            ariaLabel="Видимость фона"
          />
        </SettingsRow>

        <SettingsRow
          title="Скорость"
          description="Как быстро движется градиент — низкое значение — медленный дрейф, высокое — быстрый цикл."
        >
          <PercentSlider
            value={settings.backgroundSpeed}
            onChange={(value) => update({ backgroundSpeed: value })}
            disabled={!settings.backgroundAnimationEnabled}
            ariaLabel="Скорость потока фона"
          />
        </SettingsRow>
      </section>
    </div>
  );
}
