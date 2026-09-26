import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ColorScheme = "greenUp" | "redUp";

export type ProviderId = "binance";

/** A watchlist entry. */
export interface Instrument {
  provider: ProviderId;
  /** The provider's symbol, e.g. BTCUSDT. */
  symbol: string;
  base: string;
  quote: string;
  /** Decimals of the tick size; null lets the app pick by magnitude. */
  decimals: number | null;
  /** Shown in the menu bar title, not only in the dropdown. */
  pinned: boolean;
}

/** "binance:BTCUSDT": how windows and menus refer to an instrument. */
export const instrumentId = (instrument: Pick<Instrument, "provider" | "symbol">): string =>
  `${instrument.provider}:${instrument.symbol}`;

export interface Settings {
  watchlist: Instrument[];
  showSymbol: boolean;
  showChange: boolean;
  colorScheme: ColorScheme;
}

export interface Status {
  label: string;
  tone: "live" | "busy" | "error" | "idle";
}

export interface Candidate extends Instrument {
  /** Taken from the typed text because the instrument list was unavailable. */
  manual: boolean;
}

export interface Search {
  candidates: Candidate[];
  /** The instrument list could not be loaded, so only exact input matches. */
  degraded: boolean;
}

export type ChartMode = "line" | "candle";

export interface Interval {
  /** Seconds per candle. */
  secs: number;
  label: string;
  /** How it first shows. */
  mode: ChartMode;
}

/** What the chart window offers for an instrument. */
export interface ChartSpec {
  /** The provider's name, for messages like "cannot reach …". */
  source: string;
  intervals: Interval[];
  /** What the statistics cover, e.g. "24h". */
  statsSpan: string;
  volumeUnit: string;
  turnoverUnit: string;
  link: { label: string; url: string } | null;
}

/** Times in epoch seconds, as Liveline takes them. */
export interface Candle {
  time: number;
  open: number;
  high: number;
  low: number;
  close: number;
}

/** Over the provider's span (see ChartSpec.statsSpan). */
export interface Stats {
  last: number;
  high: number;
  low: number;
  volume: number;
  turnover: number;
  change: number;
  changePct: number;
}

export interface Level {
  price: number;
  qty: number;
}

/** Best level first on both sides. */
export interface Book {
  bids: Level[];
  asks: Level[];
}

export interface Trade {
  /** Increases with every trade of the instrument. */
  id: number;
  price: number;
  qty: number;
  /** Epoch milliseconds. */
  time: number;
  /** The taker sold. */
  sell: boolean;
}

export type FeedState = "connecting" | "live" | "offline";

export type LiveEvent =
  | { kind: "state"; state: FeedState }
  | { kind: "stats"; stats: Stats }
  | { kind: "book"; book: Book }
  /** New trades, oldest first. */
  | { kind: "trades"; trades: Trade[] };

export const MAX_INSTRUMENTS = 30;

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  getStatus: () => invoke<Status>("get_status"),
  getLoginItem: () => invoke<boolean>("get_login_item"),
  setLoginItem: (enabled: boolean) => invoke<boolean>("set_login_item", { enabled }),
  /** Tells the app the page has content, so its hidden window can be shown. */
  ready: () => invoke<void>("window_ready"),
  /** Instruments matching the query; an empty query only starts loading the lists. */
  search: (query: string) => invoke<Search>("search_instruments", { query }),
  /** The instrument the chart window should show (kept by the app, see onChartInstrument). */
  getChartInstrument: () => invoke<string | null>("get_chart_instrument"),
  openChart: (id: string) => invoke<void>("open_chart", { id }),
  chartSpec: (id: string) => invoke<ChartSpec>("chart_spec", { id }),
  /** Candles oldest first: the latest (the last still forming), or those before `end`. */
  chartHistory: (id: string, interval: number, end: number | null, limit: number) =>
    invoke<Candle[]>("chart_history", { id, interval, end, limit }),
  /** The latest trades, oldest first. */
  chartTrades: (id: string, limit: number) => invoke<Trade[]>("chart_trades", { id, limit }),
  /** Streams live data for `id` into `onEvent`; resolves to the function that stops it. */
  chartStream: async (id: string, onEvent: (event: LiveEvent) => void): Promise<() => void> => {
    const events = new Channel<LiveEvent>();
    events.onmessage = onEvent;
    const handle = await invoke<number>("chart_stream", { id, events });
    return () => void invoke("chart_stream_stop", { handle }).catch(() => {});
  },
  /** Opens the instrument's page on its provider's website. */
  openLink: (id: string) => invoke<void>("open_link", { id }),
  onStatus: (handler: (status: Status) => void): Promise<UnlistenFn> =>
    listen<Status>("status", (event) => handler(event.payload)),
  /** Saved settings, from whichever window saved them. */
  onSettings: (handler: (settings: Settings) => void): Promise<UnlistenFn> =>
    listen<Settings>("settings", (event) => handler(event.payload)),
  /** The tray asked the open chart window to show another instrument. */
  onChartInstrument: (handler: (id: string) => void): Promise<UnlistenFn> =>
    listen<string>("chart-instrument", (event) => handler(event.payload)),
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
