import { motion, type MotionValue, useTransform } from "framer-motion";
import { DOCK_ROW_DIVIDER_HEIGHT_RATIO } from "../lib/constants";

interface DockRowDividerProps {
  iconSizePx: MotionValue<number>;
  className?: string;
}

/** Vertical hairline between dock row groups — same spec for the settings
 * gear divider and user-placed in-row separators. */
export function DockRowDivider({ iconSizePx, className = "" }: DockRowDividerProps) {
  const heightPx = useTransform(
    iconSizePx,
    (px) => px * DOCK_ROW_DIVIDER_HEIGHT_RATIO,
  );

  return (
    <motion.div
      aria-hidden
      className={`mb-3 w-px shrink-0 self-end bg-zinc-600/80 ${className}`}
      style={{ height: heightPx }}
    />
  );
}
