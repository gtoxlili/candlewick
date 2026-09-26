import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ColorScheme = "greenUp" | "redUp";

export interface Coin {
  symbol: string;
  base: string;
  quote: string;
  /** Decimals of the pair's tick size; null lets the app pick by magnitude. */
  decimals: number | null;
  /** Shown in the menu bar title, not only in the dropdown. */
  pinned: boolean;
}

export interface Settings {
  coins: Coin[];
  showSymbol: boolean;
  showChange: boolean;
  colorScheme: ColorScheme;
}

export interface Status {
  label: string;
  tone: "live" | "busy" | "error" | "idle";
}

export const MAX_COINS = 30;

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  getStatus: () => invoke<Status>("get_status"),
  getLoginItem: () => invoke<boolean>("get_login_item"),
  setLoginItem: (enabled: boolean) => invoke<boolean>("set_login_item", { enabled }),
  /** Tells the app the page has content, so its hidden window can be shown. */
  ready: () => invoke<void>("window_ready"),
  /** The pair the chart window should show (kept by the app, see onChartSymbol). */
  getChartSymbol: () => invoke<string | null>("get_chart_symbol"),
  openChart: (symbol: string) => invoke<void>("open_chart", { symbol }),
  openInBinance: (symbol: string) => invoke<void>("open_in_binance", { symbol }),
  onStatus: (handler: (status: Status) => void): Promise<UnlistenFn> =>
    listen<Status>("status", (event) => handler(event.payload)),
  /** Saved settings, from whichever window saved them. */
  onSettings: (handler: (settings: Settings) => void): Promise<UnlistenFn> =>
    listen<Settings>("settings", (event) => handler(event.payload)),
  /** The tray asked the open chart window to show another pair. */
  onChartSymbol: (handler: (symbol: string) => void): Promise<UnlistenFn> =>
    listen<string>("chart-symbol", (event) => handler(event.payload)),
};

/** Subscribes for the lifetime of an effect; returns the effect cleanup. */
export function subscribe(start: Promise<UnlistenFn>): () => void {
  let unlisten: UnlistenFn | undefined;
  let disposed = false;
  void start.then((fn) => (disposed ? fn() : (unlisten = fn)));
  return () => {
    disposed = true;
    unlisten?.();
  };
}
