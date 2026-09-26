import type { ReactNode } from "react";
import { cn } from "cn";

/**
 * The window's title bar, drawn by the page under the native traffic lights
 * (TitleBarStyle::Overlay). It matches the system title bar: 32 pt tall with
 * the lights centered at 16 pt. Everything inside drags the window except
 * controls (Tauri's "deep" drag region skips buttons, inputs, selects…).
 */
export function TitleBar(props: { children?: ReactNode; className?: string; divider?: boolean }) {
  return (
    <header
      data-tauri-drag-region="deep"
      className={cn(
        "flex h-8 shrink-0 items-center select-none",
        props.divider && "border-b",
        props.className,
      )}
    >
      {props.children}
    </header>
  );
}

/** Horizontal room the traffic lights take (they end at 69 pt), plus a gap. */
export const TRAFFIC_LIGHTS_INSET = "pl-[80px]";
