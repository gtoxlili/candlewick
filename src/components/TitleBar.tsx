import type { ReactNode } from "react";
import { cn } from "cn";

// Inlined: a file would be emitted on macOS too, where this code is dropped.
import icon16 from "@/assets/window-icon-16.png?inline";
import icon32 from "@/assets/window-icon-32.png?inline";
import icon48 from "@/assets/window-icon-48.png?inline";
import { CaptionButtons } from "@/components/CaptionButtons";

/**
 * The window's title bar, drawn by the page, 32 px tall like the system's.
 * On macOS it sits under the native traffic lights (TitleBarStyle::Overlay);
 * on Windows it ends in Windows 11's caption buttons. Everything but controls
 * drags the window: Tauri's "deep" drag region on macOS, and on Windows CSS
 * `app-region` (see index.css), which WebView2 turns into a native caption:
 * snapping, double-click to maximize, the system menu on right-click.
 */
export function TitleBar(props: {
  /** A plain title: centered on macOS, beside the app icon on Windows. */
  title?: string;
  children?: ReactNode;
  className?: string;
  divider?: boolean;
  /** Windows: the window can be maximized, so it gets the button for it. */
  maximizable?: boolean;
}) {
  return (
    <header
      data-tauri-drag-region="deep"
      className={cn(
        "titlebar flex h-8 shrink-0 items-center select-none",
        props.divider && "border-b",
        props.className,
      )}
    >
      {props.title !== undefined &&
        (__WINDOWS__ ? (
          <div className="flex min-w-0 flex-1 items-center gap-3 pl-4">
            <img
              src={icon16}
              srcSet={`${icon16} 1x, ${icon32} 2x, ${icon48} 3x`}
              alt=""
              draggable={false}
              className="size-4 shrink-0"
            />
            <h1 className="truncate text-xs">{props.title}</h1>
          </div>
        ) : (
          <h1 className="flex-1 text-center text-sm font-semibold">{props.title}</h1>
        ))}
      {props.children}
      {__WINDOWS__ && <CaptionButtons maximizable={props.maximizable ?? false} />}
    </header>
  );
}

/**
 * Where title bar content starts: past the traffic lights on macOS (they end
 * at 69 pt, plus a gap), 16 px in on Windows, as its title bars do.
 */
export const TITLE_INSET = __WINDOWS__ ? "pl-4" : "pl-[80px]";
