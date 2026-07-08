import { motion, type MotionValue, useTransform } from "framer-motion";
import {
  DOCK_ROW_DIVIDER_HEIGHT_RATIO,
  LED_HEIGHT_PX,
  getSizeMetrics,
} from "../lib/constants";

interface DockRowDividerProps {
  iconSizePx: MotionValue<number>;
  /** When true (Left/Right dock), the divider is a horizontal hairline. */
  isVertical?: boolean;
  className?: string;
}

const DIVIDER_COLOR = "rgb(82 82 91 / 0.8)";

/** Hairline between dock row groups — perpendicular to the icon layout axis. */
export function DockRowDivider({
  iconSizePx,
  isVertical = false,
  className = "",
}: DockRowDividerProps) {
  const lineSizePx = useTransform(
    iconSizePx,
    (px) => px * DOCK_ROW_DIVIDER_HEIGHT_RATIO,
  );
  /** Lifts the hairline's bottom to the icons' bottom edge on horizontal
   * docks — the `self-end` baseline is the LED bar's bottom, so the offset
   * is the same scaled gap + LED height the icon column itself uses (was a
   * fixed `mb-3`, which only matched the 56px default size). */
  const endOffsetPx = useTransform(
    iconSizePx,
    (px) => getSizeMetrics(px).iconLedGapPx + LED_HEIGHT_PX,
  );

  return (
    <motion.div
      aria-hidden
      className={`shrink-0 grow-0 ${
        isVertical ? "mx-3 self-center" : "self-end"
      } ${className}`}
      style={
        isVertical
          ? {
              width: lineSizePx,
              height: 1,
              minHeight: 1,
              maxHeight: 1,
              backgroundColor: DIVIDER_COLOR,
            }
          : {
              width: 1,
              minWidth: 1,
              maxWidth: 1,
              height: lineSizePx,
              marginBottom: endOffsetPx,
              backgroundColor: DIVIDER_COLOR,
            }
      }
    />
  );
}
