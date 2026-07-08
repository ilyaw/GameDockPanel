import { motion, type MotionValue, useTransform } from "framer-motion";
import { DOCK_ROW_DIVIDER_HEIGHT_RATIO } from "../lib/constants";

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

  return (
    <motion.div
      aria-hidden
      className={`shrink-0 grow-0 ${
        isVertical ? "mx-3 self-center" : "mb-3 self-end"
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
              backgroundColor: DIVIDER_COLOR,
            }
      }
    />
  );
}
