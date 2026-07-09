import { useState, type MouseEventHandler, type ReactNode } from "react";

interface DockContextMenuRowProps {
  as?: "button" | "div";
  disabled?: boolean;
  muted?: boolean;
  onClick?: MouseEventHandler<HTMLButtonElement>;
  /** Fired alongside the row's own hover highlight — use when a parent
   * cannot rely on bubbled/non-bubbled enter events (WKWebView menus). */
  onRowMouseEnter?: () => void;
  onRowMouseLeave?: () => void;
  className?: string;
  children: ReactNode;
}

/** Context menu row with JS-driven hover — CSS :hover is flaky in Tauri WKWebView. */
export function DockContextMenuRow({
  as: Tag = "button",
  disabled = false,
  muted = false,
  onClick,
  onRowMouseEnter,
  onRowMouseLeave,
  className = "",
  children,
}: DockContextMenuRowProps) {
  const [hovered, setHovered] = useState(false);

  const baseClassName = `flex w-full items-center gap-2 px-3 py-1.5 text-left ${
    disabled
      ? "cursor-not-allowed opacity-40"
      : hovered
        ? "bg-zinc-800"
        : ""
  } ${
    muted ? (hovered && !disabled ? "text-zinc-200" : "text-zinc-400") : ""
  } ${className}`;

  const pointerHandlers = {
    onMouseEnter: () => {
      if (!disabled) setHovered(true);
      onRowMouseEnter?.();
    },
    onMouseLeave: () => {
      setHovered(false);
      onRowMouseLeave?.();
    },
  };

  if (Tag === "div") {
    return (
      <div className={baseClassName} {...pointerHandlers}>
        {children}
      </div>
    );
  }

  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={baseClassName}
      {...pointerHandlers}
    >
      {children}
    </button>
  );
}
