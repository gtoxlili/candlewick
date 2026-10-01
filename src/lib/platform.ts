// What differs between the macOS and Windows pages. The platform is fixed at
// build time (`__WINDOWS__`), since each platform builds its own copy of the
// pages; what only the running system knows (on Windows, the backdrop behind
// the window) comes from the app before the first render.

/** Where the pinned entry shows. */
export const BAR = __WINDOWS__ ? "任务栏" : "菜单栏";

/** Set by the app before the page loads (platform/windows/window.rs). */
interface Chrome {
  backdrop: "mica" | "solid";
}

declare global {
  interface Window {
    __CANDLEWICK__?: Chrome;
  }
}

/**
 * Prepares the document before the first render: `data-platform` on the root
 * element for the platform's styles, and on Windows the backdrop and the
 * window's active state.
 */
export function setupPlatform(): void {
  const root = document.documentElement;
  root.dataset.platform = __WINDOWS__ ? "windows" : "macos";
  if (!__WINDOWS__) return;

  // Mica needs Windows 11; before that the page paints Fluent's solid background.
  root.dataset.backdrop = window.__CANDLEWICK__?.backdrop ?? "solid";

  // Title bar text and caption buttons fade while another window is active,
  // as in Windows' own title bars.
  const active = () => {
    root.dataset.active = String(document.hasFocus());
  };
  window.addEventListener("focus", active);
  window.addEventListener("blur", active);
  active();

  // WebView2's own menu (Back, Refresh, Print, Inspect) has no place in an app
  // window; text fields keep theirs for cut, copy and paste.
  document.addEventListener("contextmenu", (event) => {
    const target = event.target;
    const editable =
      target instanceof HTMLElement &&
      (target.isContentEditable || target.closest("input, textarea, [contenteditable]") !== null);
    if (!editable) event.preventDefault();
  });
}
