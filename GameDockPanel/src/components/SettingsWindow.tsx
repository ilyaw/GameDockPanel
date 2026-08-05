import { useCallback, useEffect, useRef, useState, type ChangeEvent, type CSSProperties, type ReactNode } from "react";
import { emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useDockSettings } from "../hooks/useDockSettings";
import { useLaunchAtLogin } from "../hooks/useLaunchAtLogin";
import {
  BACKGROUND_PRESETS,
  BORDER_STYLE_PRESETS,
  BORDER_WIDTH_MAX_PX,
  BORDER_WIDTH_MIN_PX,
  DOCK_POSITION_OPTIONS,
  DOCK_WINDOW_LAYER_OPTIONS,
  ICON_SIZE_PRESETS,
  ICON_SIZE_MAX_PX,
  ICON_SIZE_MIN_PX,
  LED_COLOR_MODE_OPTIONS,
  PANEL_EFFECT_PRESETS,
  clampBorderWidthPx,
  clampIconSizePx,
  type BackgroundPreset,
  type BorderStylePreset,
  type IconSizePreset,
  type LedColorMode,
  type PanelEffectPreset,
} from "../lib/constants";
import type { DockSettings } from "../lib/types";
import { IS_WINDOWS } from "../lib/windowsDock";

type SettingsTabId = "panel" | "indicators" | "border" | "background" | "system";

const SETTINGS_TABS: { id: SettingsTabId; label: string }[] = [
  { id: "panel", label: "Панель" },
  { id: "indicators", label: "Индикаторы" },
  { id: "border", label: "Рамка" },
  { id: "background", label: "Фон" },
  { id: "system", label: "Система" },
];

type RangeStyle = CSSProperties & Record<"--settings-range-progress", string>;

function rangeProgressStyle(value: number, min: number, max: number): RangeStyle {
  const span = max - min;
  const pct = span <= 0 ? 0 : ((value - min) / span) * 100;
  return { "--settings-range-progress": `${pct}%` };
}

/** Visible gaming-style range — track/fill styled in `index.css` (`.settings-range`). */
function SettingsRange({
  value,
  min,
  max,
  step,
  disabled,
  onChange,
  onPointerUp,
  onKeyUp,
  ariaLabel,
  className = "settings-range--compact",
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  onChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onPointerUp?: () => void;
  onKeyUp?: () => void;
  ariaLabel: string;
  className?: string;
}) {
  return (
    <input
      type="range"
      min={min}
      max={max}
      step={step}
      value={value}
      disabled={disabled}
      onChange={onChange}
      onPointerUp={onPointerUp}
      onKeyUp={onKeyUp}
      style={rangeProgressStyle(value, min, max)}
      className={`settings-range ${className} disabled:cursor-not-allowed`}
      aria-label={ariaLabel}
    />
  );
}

/** Minimal accessible on/off switch — no toggle primitive in lucide-react,
 * and pulling in a UI library for one control isn't warranted here. */
function ToggleSwitch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`settings-window__toggle relative h-6 w-11 shrink-0 rounded-full transition-[background,box-shadow] disabled:cursor-not-allowed disabled:opacity-50 ${
        checked ? "settings-window__toggle--on" : ""
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
      <SettingsRange
        min={0}
        max={100}
        value={Math.round(value * 100)}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value) / 100)}
        ariaLabel={ariaLabel}
      />
      <span className="settings-window__value w-9 text-right text-xs tabular-nums">
        {Math.round(value * 100)}%
      </span>
    </div>
  );
}

/** Integer px slider for the RGB frame thickness (1–8 px). */
function PxSlider({
  value,
  min,
  max,
  onChange,
  disabled,
  ariaLabel,
}: {
  value: number;
  min: number;
  max: number;
  onChange: (next: number) => void;
  disabled?: boolean;
  ariaLabel: string;
}) {
  return (
    <div className="flex w-36 items-center gap-3">
      <SettingsRange
        min={min}
        max={max}
        step={1}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value))}
        ariaLabel={ariaLabel}
      />
      <span className="settings-window__value w-9 text-right text-xs tabular-nums">
        {value} px
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
        selected
          ? "scale-105 border-cyan-300 shadow-[0_0_14px_-2px_rgb(34_211_238/55%)]"
          : "border-zinc-600/80 hover:scale-105 hover:border-zinc-500"
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
      className={`settings-window__preset rounded-lg px-2.5 py-1 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-40 ${
        selected ? "settings-window__preset--active" : ""
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
        <p className="settings-window__row-title text-sm font-medium">{title}</p>
        {description && (
          <p className={`settings-window__row-desc mt-0.5 text-xs ${descriptionClassName}`}>
            {description}
          </p>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function SettingsTabBar({
  activeTab,
  onChange,
}: {
  activeTab: SettingsTabId;
  onChange: (tab: SettingsTabId) => void;
}) {
  return (
    <nav
      className="settings-window__tabs mb-4 flex gap-1 overflow-x-auto pb-px"
      aria-label="Разделы настроек"
    >
      {SETTINGS_TABS.map((tab) => {
        const selected = activeTab === tab.id;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={selected}
            onClick={() => onChange(tab.id)}
            className={`settings-window__tab shrink-0 rounded-t-lg px-3 py-2 text-sm font-medium ${
              selected ? "settings-window__tab--active" : ""
            }`}
          >
            {tab.label}
          </button>
        );
      })}
    </nav>
  );
}

function SettingsSectionCard({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="settings-window__card rounded-xl">
      <header className="settings-window__card-header px-4 py-3">
        <h2 className="text-sm font-semibold text-zinc-100">{title}</h2>
        {description && (
          <p className="settings-window__row-desc mt-0.5 text-xs">{description}</p>
        )}
      </header>
      <div className="divide-y divide-zinc-700/60 px-4">{children}</div>
    </section>
  );
}

function DisabledHint({ show, children }: { show: boolean; children: ReactNode }) {
  if (!show) return null;
  return <p className="mt-1 text-right text-[11px] text-violet-300/70">{children}</p>;
}

const colorInputClass =
  "settings-window__color-input h-8 w-8 cursor-pointer rounded bg-transparent p-0 disabled:cursor-not-allowed";

export function SettingsWindow() {
  const { settings, commit } = useDockSettings();
  const {
    enabled: launchAtLoginEnabled,
    busy: launchAtLoginBusy,
    error: launchAtLoginError,
    toggle: toggleLaunchAtLogin,
  } = useLaunchAtLogin();
  const [activeTab, setActiveTab] = useState<SettingsTabId>("panel");
  const [sliderIconSizePx, setSliderIconSizePx] = useState(settings.iconSizePx);
  const [indicatorColorsRefreshing, setIndicatorColorsRefreshing] = useState(false);
  const [indicatorColorsRefreshNote, setIndicatorColorsRefreshNote] = useState<string | null>(
    null,
  );
  const [diagnosticsCopied, setDiagnosticsCopied] = useState(false);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [diagSnapNote, setDiagSnapNote] = useState<string | null>(null);
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

  const refreshIndicatorColors = useCallback(async () => {
    if (indicatorColorsRefreshing) return;

    if (settings.ledColorMode !== "auto") {
      setIndicatorColorsRefreshNote(
        "Сначала выберите режим «Из иконки» — в остальных режимах пересчёт не меняет LED на доке.",
      );
      return;
    }

    setIndicatorColorsRefreshing(true);
    setIndicatorColorsRefreshNote(null);
    try {
      await invoke("refresh_indicator_colors");
      setIndicatorColorsRefreshNote(
        "Готово. Цвета обновлены — смотрите LED под иконкой у запущенного приложения или в ПКМ → «Цвет индикатора».",
      );
    } catch (error: unknown) {
      console.error("Failed to refresh indicator colors:", error);
      setIndicatorColorsRefreshNote("Не удалось пересчитать цвета. Подробности — в консоли разработчика.");
    } finally {
      setIndicatorColorsRefreshing(false);
    }
  }, [indicatorColorsRefreshing, settings.ledColorMode]);

  const copyDiagnostics = useCallback(async () => {
    setDiagnosticsError(null);
    try {
      const payload = await invoke<Record<string, unknown>>("get_diagnostics");
      await navigator.clipboard.writeText(JSON.stringify(payload, null, 2));
      setDiagnosticsCopied(true);
      setTimeout(() => setDiagnosticsCopied(false), 2000);
    } catch (error: unknown) {
      console.error("Failed to copy diagnostics:", error);
      setDiagnosticsError("Не удалось собрать диагностику.");
    }
  }, []);

  const openLogDir = useCallback(async () => {
    setDiagnosticsError(null);
    try {
      await invoke("open_log_dir");
    } catch (error: unknown) {
      console.error("Failed to open log dir:", error);
      setDiagnosticsError("Не удалось открыть папку логов.");
    }
  }, []);

  const logWindowsDiagNow = useCallback(async () => {
    setDiagnosticsError(null);
    setDiagSnapNote(null);
    try {
      const snap = await invoke<Record<string, unknown>>("log_windows_diag");
      console.info("[win-diag] manual snapshot", snap);
      const healthy = snap.healthy === true;
      const issues = Array.isArray(snap.healthIssues)
        ? (snap.healthIssues as string[]).join(", ")
        : "";
      setDiagSnapNote(
        healthy
          ? "Снимок записан в лог (HEALTHY)."
          : `Снимок в лог: BAD — ${issues || "см. консоль"}`,
      );
      setTimeout(() => setDiagSnapNote(null), 4000);
    } catch (error: unknown) {
      console.error("Failed to log windows diag:", error);
      setDiagnosticsError("Не удалось записать win-diag снимок.");
    }
  }, []);

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

  const panelTab = (
    <SettingsSectionCard
      title="Панель"
      description="Положение, размер иконок и поведение при наведении."
    >
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
            <div className="mb-1.5 flex justify-between text-[10px] text-zinc-400">
              <span>Компакт</span>
              <span>Крупный</span>
            </div>
            <div className="flex items-center gap-3">
              <SettingsRange
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
                className="settings-range--fluid"
                ariaLabel="Размер иконок"
              />
              <span className="settings-window__value w-10 shrink-0 text-right text-xs tabular-nums">
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
        title="Увеличение соседних иконок"
        descriptionClassName="max-w-sm"
        description="Насколько сильно при наведении растут иконки рядом с той, под которой курсор. На минимуме увеличивается только активная иконка."
      >
        <div className="flex w-full max-w-[280px] flex-col items-end gap-1.5">
          <div className="mb-0.5 flex w-full justify-between text-[10px] text-zinc-400">
            <span>Только активная</span>
            <span>Максимум</span>
          </div>
          <PercentSlider
            value={settings.magnifyNeighborStrength}
            onChange={(value) => update({ magnifyNeighborStrength: value })}
            ariaLabel="Увеличение соседних иконок"
          />
        </div>
      </SettingsRow>
    </SettingsSectionCard>
  );

  const indicatorsTab = (
    <SettingsSectionCard
      title="Индикаторы"
      description="Цвет полоски под иконкой запущенного приложения."
    >
      <SettingsRow
        title="Режим цвета"
        description="Автоматически из иконки, один для всех, или только вручную заданные."
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
            disabled={indicatorColorsRefreshing}
            onClick={() => {
              void refreshIndicatorColors();
            }}
            className="text-xs text-cyan-300/80 underline-offset-2 hover:text-cyan-200 hover:underline disabled:cursor-wait disabled:opacity-60"
          >
            {indicatorColorsRefreshing ? "Пересчитываем…" : "Пересчитать цвета из иконок"}
          </button>
          <p className="max-w-[240px] text-right text-[11px] leading-snug text-zinc-500">
            LED виден только у запущенных приложений. Ручные цвета из контекстного меню не
            сбрасываются.
          </p>
          {indicatorColorsRefreshNote && (
            <p className="max-w-[240px] text-right text-[11px] leading-snug text-cyan-300/80">
              {indicatorColorsRefreshNote}
            </p>
          )}
        </div>
      </SettingsRow>
    </SettingsSectionCard>
  );

  const borderTab = (
    <SettingsSectionCard
      title="Рамка"
      description="RGB-перелив, стиль анимации и цвета кольца по периметру панели."
    >
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
        title="Стиль рамки"
        description="Как анимируется RGB-рамка: классические спектр/глитч/скан или spotlight-потоки (вращение, sweep, неон-пульс)."
      >
        <div className="flex flex-col items-end">
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
          <DisabledHint show={!settings.animationsEnabled}>
            Включите анимации рамки выше
          </DisabledHint>
        </div>
      </SettingsRow>

      <SettingsRow
        title="Толщина рамки"
        description="Ширина кольца по всему периметру. Для заметной «бегущей линии» (Скан, Спектр, Поток) — 5–8 px."
      >
        <PxSlider
          value={clampBorderWidthPx(settings.borderWidthPx)}
          min={BORDER_WIDTH_MIN_PX}
          max={BORDER_WIDTH_MAX_PX}
          onChange={(value) => update({ borderWidthPx: value })}
          disabled={!settings.animationsEnabled}
          ariaLabel="Толщина рамки"
        />
      </SettingsRow>

      <SettingsRow
        title="Цвета RGB-рамки"
        description="Опорные цвета переливающейся рамки."
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
          <DisabledHint show={!settings.animationsEnabled}>
            Включите анимации рамки выше
          </DisabledHint>
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
    </SettingsSectionCard>
  );

  const backgroundTab = (
    <SettingsSectionCard
      title="Фон и эффекты"
      description="Декоративные слои поверх стеклянной панели."
    >
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
          <DisabledHint show={!settings.panelEffectEnabled}>
            Включите эффект панели выше
          </DisabledHint>
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
        <div className="flex flex-col items-end">
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
          <DisabledHint show={!settings.backgroundAnimationEnabled}>
            Включите анимацию фона выше
          </DisabledHint>
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
    </SettingsSectionCard>
  );

  const systemTab = (
    <SettingsSectionCard
      title="Система"
      description="Поведение приложения и диагностика."
    >
      <SettingsRow
        title="Слой отображения"
        descriptionClassName="max-w-sm"
        description="Поверх окон — панель всегда видна; под окнами — окна приложений перекрывают панель, как в RocketDock."
      >
        <div className="flex max-w-[240px] flex-wrap justify-end gap-1.5">
          {DOCK_WINDOW_LAYER_OPTIONS.map((option) => (
            <StylePresetButton
              key={option.id}
              preset={{
                id: option.id,
                label: option.label,
                description: option.description,
              }}
              selected={settings.dockWindowLayer === option.id}
              onSelect={() => update({ dockWindowLayer: option.id })}
            />
          ))}
        </div>
      </SettingsRow>

      <SettingsRow
        title="Запуск при входе в систему"
        descriptionClassName="max-w-sm"
        description={
          IS_WINDOWS
            ? "Добавляет приложение в автозагрузку — док запустится автоматически при входе в Windows."
            : "Добавляет приложение в Login Items — док запустится автоматически при входе в macOS."
        }
      >
        <div className="flex flex-col items-end gap-1.5">
          <ToggleSwitch
            checked={launchAtLoginEnabled ?? false}
            disabled={launchAtLoginBusy || launchAtLoginEnabled === null}
            onChange={(value) => {
              void toggleLaunchAtLogin(value);
            }}
          />
          {launchAtLoginError && (
            <p className="max-w-xs text-right text-xs text-red-400">
              {launchAtLoginError}
            </p>
          )}
        </div>
      </SettingsRow>

      <SettingsRow
        title="Диагностика"
        descriptionClassName="max-w-sm"
        description="Для отладки с другом: 1) «Скопировать диагностику» → вставь JSON в чат 2) «Открыть папку логов» → пришли файл gamedockpanel*.log 3) скрин панели (лучше с оверлеем). На Windows логи: %LOCALAPPDATA%\\com.ilya.gamedockpanel\\logs. Ищи [win-diag] scale/dpr_js и строку DPI_MISMATCH (advisory, не chrome BAD)."
      >
        <div className="flex flex-col items-end gap-2">
          <div className="flex flex-wrap justify-end gap-2">
            <button
              type="button"
              className="settings-window__btn rounded-md px-3 py-1.5 text-xs"
              onClick={() => {
                void copyDiagnostics();
              }}
            >
              {diagnosticsCopied ? "Скопировано" : "Скопировать диагностику"}
            </button>
            <button
              type="button"
              className="settings-window__btn rounded-md px-3 py-1.5 text-xs"
              onClick={() => {
                void openLogDir();
              }}
            >
              Открыть папку логов
            </button>
            {IS_WINDOWS && (
              <button
                type="button"
                className="settings-window__btn rounded-md px-3 py-1.5 text-xs"
                onClick={() => {
                  void logWindowsDiagNow();
                }}
              >
                Снимок в лог
              </button>
            )}
          </div>
          {diagSnapNote && (
            <p className="max-w-xs text-right text-xs text-emerald-400">{diagSnapNote}</p>
          )}
          {diagnosticsError && (
            <p className="max-w-xs text-right text-xs text-red-400">{diagnosticsError}</p>
          )}
        </div>
      </SettingsRow>

      {IS_WINDOWS && (
        <SettingsRow
          title="Оверлей отладки Windows"
          descriptionClassName="max-w-sm"
          description="На доке: пурпурная рамка = HWND, голубая = CSS-пилюля, красный фон = chrome сломан (CAPTION / нет LAYERED / Δ). В лог каждые 2с пишется [win-diag]. Оставь включённым и пришли скрин + лог."
        >
          <ToggleSwitch
            checked={Boolean(settings.windowsDebugOverlay)}
            onChange={(value) => {
              update({ windowsDebugOverlay: value });
            }}
          />
        </SettingsRow>
      )}

      {IS_WINDOWS && (
        <SettingsRow
          title="Жёсткая обрезка окна (совместимость)"
          descriptionClassName="max-w-sm"
          description="Старый режим: углы дока вырезаются системной GDI-маской (грубее край, зато прячет непрозрачные углы, если прозрачность на этой машине сломана). По умолчанию выключено — углы скругляет сам рендер, картинка чётче."
        >
          <ToggleSwitch
            checked={Boolean(settings.windowsHardClip)}
            onChange={(value) => {
              update({ windowsHardClip: value });
            }}
          />
        </SettingsRow>
      )}
    </SettingsSectionCard>
  );

  const tabContent: Record<SettingsTabId, ReactNode> = {
    panel: panelTab,
    indicators: indicatorsTab,
    border: borderTab,
    background: backgroundTab,
    system: systemTab,
  };

  return (
    <div
      className={`settings-window flex h-screen w-full flex-col${
        IS_WINDOWS ? " settings-window--windows" : ""
      }`}
    >
      <header className="settings-window__header shrink-0 px-6 py-4">
        <h1 className="settings-window__title text-lg font-semibold">Настройки</h1>
        <p className="mt-0.5 text-xs tracking-wide text-violet-300/70 uppercase">
          GameDockPanel
        </p>
      </header>

      <div className="flex min-h-0 flex-1 flex-col px-6 py-4">
        <SettingsTabBar activeTab={activeTab} onChange={setActiveTab} />

        <div className="min-h-0 flex-1 overflow-y-auto pb-4" role="tabpanel">
          {tabContent[activeTab]}
        </div>
      </div>
    </div>
  );
}
