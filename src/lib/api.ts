import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { Language, Locale } from "@/lib/i18n";

export type ColorScheme = "greenUp" | "redUp";

/** The crypto exchanges; the settings pick one for every pair. */
export type Exchange = "binance" | "bybit" | "okx";

/** Where stocks come from: Longbridge (US, Hong Kong, China), Tonghuashun (China). */
type StockSource = "longbridge" | "ths";

/** Exchanges, the stock sources, DEX Screener for on-chain pools. */
export type ProviderId = Exchange | StockSource | "dexscreener";

export const EXCHANGES = ["binance", "bybit", "okx"] as const satisfies readonly Exchange[];

const isStock = (provider: ProviderId): provider is StockSource => provider === "longbridge" || provider === "ths";

/** A watchlist entry. */
export interface Instrument {
  provider: ProviderId;
  /** The provider's symbol, e.g. BTCUSDT, BTC-USDT, AAPL.US, solana/EP2ib6…. */
  symbol: string;
  /** Crypto: the base asset (BTC); a pool: its token ($WIF). Stocks: the code (AAPL, 700). */
  base: string;
  /** Crypto: the quote asset (USDT); a pool: USD. Stocks: the currency. */
  quote: string;
  /** A stock's or token's name, e.g. 腾讯控股, dogwifhat. */
  name?: string | null;
  /** Decimals of the tick size; null lets the app pick by magnitude. */
  decimals: number | null;
  /** Shown in the menu bar or taskbar, not only in the dropdown. */
  pinned: boolean;
  /** Where a pool trades. */
  venue?: Venue | null;
}

/** A pool's chain and DEX, by name. */
export interface Venue {
  /** "Solana", "BNB Chain" */
  chain: string;
  /** "Raydium", "Uniswap" */
  dex?: string | null;
}

/** "binance:BTCUSDT": how windows and menus refer to an instrument. */
export const instrumentId = (instrument: Pick<Instrument, "provider" | "symbol">): string =>
  `${instrument.provider}:${instrument.symbol}`;

/**
 * The name, the dimmed part after it (see `Instrument::row_label` in the
 * app), and for a pool where it trades: "BTC" "/USDT"; "$WIF" " dogwifhat"
 * "Solana · Raydium"; "AAPL" " Apple"; "腾讯控股" " 700".
 */
export function instrumentLabel(instrument: Instrument): { name: string; detail: string; venue: string | null } {
  if (instrument.provider === "dexscreener") {
    const venue = [instrument.venue?.chain, instrument.venue?.dex].filter((p): p is string => !!p).join(" · ");
    return { name: instrument.base, detail: instrument.name ? ` ${instrument.name}` : "", venue: venue || null };
  }
  if (!isStock(instrument.provider)) return { name: instrument.base, detail: `/${instrument.quote}`, venue: null };
  const name = instrument.name ?? "";
  // US tickers read better than their names; other codes are just numbers.
  if (instrument.symbol.endsWith(".US") || !name) return { name: instrument.base, detail: name && ` ${name}`, venue: null };
  return { name, detail: ` ${instrument.base}`, venue: null };
}

/** Where stocks trade. */
export type Market = "US" | "HK" | "CN";

const MARKETS: Record<string, Market> = { US: "US", HK: "HK", SH: "CN", SZ: "CN" };

/** The market a stock trades in, by its symbol's suffix; none for crypto. Tonghuashun's are all China's. */
export function marketOf(instrument: Instrument): Market | null {
  if (instrument.provider === "ths") return "CN";
  return instrument.provider === "longbridge" ? (MARKETS[instrument.symbol.split(".").at(-1) ?? ""] ?? null) : null;
}

export interface Settings {
  /** Where every pair comes from; picking another exchange takes the pairs off the watchlist. */
  exchange: Exchange;
  watchlist: Instrument[];
  showSymbol: boolean;
  showChange: boolean;
  /** The bar shows the total holdings instead of a pinned entry (which the app unpins). */
  holdingsInBar: boolean;
  colorScheme: ColorScheme;
  /** Check for, download and apply updates on its own. */
  autoUpdate: boolean;
  /** AI agents may read the app's data through its local API and skill. */
  agentAccess: boolean;
  /** What the app speaks; the page hears it as `onLocale`. */
  language: Language;
}

/** Where the skill went, while AI access is on. */
export interface AgentStatus {
  /** Access has been turned on (the switch may lead it by a moment). */
  on: boolean;
  /** The agents that have it: "Claude Code", "Codex", "OpenCode". */
  agents: string[];
  error: string | null;
}

/** What the updater is doing (update.rs). */
export type UpdateState =
  | { kind: "disabled" }
  | { kind: "idle"; checked: boolean }
  | { kind: "checking" }
  | { kind: "unreachable" }
  | { kind: "downloading"; version: string }
  | { kind: "ready"; version: string }
  | { kind: "restarting"; version: string }
  | { kind: "failed"; version: string };

export interface Update {
  /** The running version. */
  current: string;
  state: UpdateState;
}

export interface Candidate extends Instrument {
  /** Taken from the typed text because the instrument list was unavailable. */
  manual: boolean;
  /** A pool's liquidity in USD, to tell the pools of one token apart. */
  liquidity: number | null;
}

export interface Search {
  candidates: Candidate[];
  /** Why results may be missing, e.g. a list that could not be loaded. */
  notes: string[];
}

export type ChartMode = "line" | "candle";

export interface Interval {
  /** Seconds per candle. */
  secs: number;
  /** How it first shows. */
  mode: ChartMode;
  /** Candles line up with the clock, so trades can be bucketed here; otherwise new candles come from the app. */
  aligned: boolean;
  /** Trades outside the regular session don't count. */
  regularOnly: boolean;
}

/** What the statistics cover: the last 24 hours (crypto), or the trading day. */
export type StatsSpan = "rolling24h" | "today";

/** What the chart window offers for an instrument. */
export interface ChartSpec {
  source: ProviderId;
  intervals: Interval[];
  statsSpan: StatsSpan;
  /** The base asset volume counts in; null for shares. */
  volumeUnit: string | null;
  turnoverUnit: string;
  /** Seconds east of UTC at which daily candles open, for their date labels. */
  dayOffset: number;
  /** Price steps the book can be grouped by, finest first. Empty: the book comes as it is. */
  bookSteps: number[];
  /** The stream carries an order book; without one the window shows the trades alone. */
  book: boolean;
  /** The statistics are a pool's: volume in the quote currency only, with liquidity, market cap and swap counts beside. */
  onchain: boolean;
  /** The instrument's page on the provider's website. */
  link: string | null;
}

/** Times in epoch seconds, as Liveline takes them. */
export interface Candle {
  time: number;
  open: number;
  high: number;
  low: number;
  close: number;
}

/** Over the provider's span (see ChartSpec.statsSpan); null where the provider gives none. */
export interface Stats {
  last: number;
  high: number | null;
  low: number | null;
  /** In the base asset (ChartSpec.volumeUnit). */
  volume: number | null;
  turnover: number;
  change: number;
  changePct: number;
  /** What a pool adds; null for exchanges and stocks. */
  onchain: Onchain | null;
}

/** A pool's standing over the same span. */
export interface Onchain {
  /** USD in the pool. */
  liquidity: number | null;
  marketCap: number | null;
  /** Fully diluted valuation, USD. */
  fdv: number | null;
  /** Swaps buying and selling the token. */
  buys: number;
  sells: number;
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
  /** Outside the regular session (US pre-market, post-market, overnight). */
  extended: boolean;
}

export type FeedState = "connecting" | "live" | "offline";

export type LiveEvent =
  | { kind: "state"; state: FeedState }
  | { kind: "stats"; stats: Stats }
  /** The book grouped by each of ChartSpec.bookSteps in turn, or just the book when there are none. */
  | { kind: "book"; books: Book[] }
  /** New trades, oldest first. */
  | { kind: "trades"; trades: Trade[] }
  /** A price without a trade behind it: moves the forming candle, not the trade list. `time` in epoch ms. */
  | { kind: "tick"; price: number; time: number };

/** What a Longbridge account may see; packages and notes as Longbridge words them. */
export interface LongbridgeAccount {
  markets: { market: Market; packages: string[]; note: string | null }[];
}

export interface Longbridge {
  /** The app key's ends, when credentials are saved. */
  appKey: string | null;
  /** As of the last login with these credentials. */
  account: LongbridgeAccount | null;
}

export interface LongbridgeKeys {
  appKey: string;
  appSecret: string;
  accessToken: string;
}

/** Tonghuashun as the settings window shows it. */
export interface Ths {
  /** The key's ends, when one is saved. */
  key: string | null;
  /** Signed in on Tonghuashun's site: the dropdown shows the watchlist there. */
  account: ThsAccount | null;
}

/** A Tonghuashun login and the watchlist read with it. */
export interface ThsAccount {
  user: string;
  /** Epoch seconds the login runs out at, where known. */
  expires: number | null;
  /** Entries in the dropdown, and those nothing here quotes (Hong Kong, US, futures). */
  count: number;
  skipped: number;
  /** Epoch ms of the last good read. */
  updated: number | null;
  /** `signedOut`: the login ran out; only signing in again helps. */
  state: "reading" | "read" | "signedOut" | "failed";
  /** Why the last read failed. */
  error: string | null;
}

/** What a CoinGecko key is, once CoinGecko has been asked. */
export type CoinGeckoAccess =
  /** A Demo key: 100 calls a minute and 10,000 a month, which it can't report. */
  | { tier: "demo" }
  /** A paid plan and its usage. */
  | { tier: "paid"; name: string; perMinute: number; monthly: number; remaining: number };

/** The CoinGecko key as the settings window shows it. */
export interface CoinGecko {
  /** The key's ends, when one is saved. */
  key: string | null;
  /** What the key is, once probed. */
  access: CoinGeckoAccess | null;
}

/** An exchange API key, as the user enters it; the app only ever reads with it. */
export interface ApiKey {
  key: string;
  secret: string;
  /** OKX's. */
  passphrase?: string;
}

/** An exchange's saved key, as the settings window shows it. */
export interface ExchangeKey {
  exchange: Exchange;
  /** The key's ends, when one is saved. */
  key: string | null;
}

export type Wallet = "spot" | "trading" | "funding" | "earn" | "usdFutures" | "coinFutures";

/** Some of an asset in one wallet of one exchange. */
export interface Holding {
  exchange: Exchange;
  wallet: Wallet;
  amount: number;
}

/** An asset across every exchange's wallets; amounts in USDT. */
export interface HeldAsset {
  asset: string;
  amount: number;
  /** null without a USDT pair or the exchange's own value. */
  price: number | null;
  value: number | null;
  /** What the 24h price move made of this amount, in USDT. */
  change: number | null;
  /** The 24h price change, in percent. */
  changePct: number | null;
  /** A dollar stablecoin: cash. */
  stable: boolean;
  /** Where it sits, largest first. */
  held: Holding[];
  /** Average cost in USD, where the exchange keeps one (OKX). */
  cost: number | null;
  pnl: number | null;
}

/** What a position is: what it settles in, and whether it expires. */
export type PositionKind =
  | "usdtPerpetual"
  | "usdtFutures"
  | "usdcPerpetual"
  | "usdcFutures"
  | "coinPerpetual"
  | "coinFutures"
  | "option"
  | "margin"
  | "other";

/** An open derivatives position. */
export interface HeldPosition {
  exchange: Exchange;
  symbol: string;
  kind: PositionKind;
  long: boolean;
  size: number;
  /** The base asset, or null for a number of contracts. */
  sizeUnit: string | null;
  entry: number;
  mark: number;
  liquidation: number | null;
  leverage: number | null;
  isolated: boolean;
  /** Unrealized, in pnlAsset. */
  pnl: number;
  pnlAsset: string;
  pnlUsd: number | null;
  /** Exposure in USD at the mark price, negative when short. */
  exposure: number;
  /** What the contract's 24h move made of the position, in USDT. */
  change: number | null;
}

export interface WalletValue {
  wallet: Wallet;
  value: number;
}

/** One exchange's part of the portfolio. */
export interface ExchangeAccount {
  exchange: Exchange;
  /** null before the first good refresh. */
  total: number | null;
  change: number | null;
  wallets: WalletValue[];
  /** Epoch milliseconds of the last good refresh. */
  updated: number | null;
  /** Why the last refresh failed. */
  error: string | null;
}

/** Every exchange with a key, as one picture. */
export interface Portfolio {
  accounts: ExchangeAccount[];
  /** USDT, over the accounts refreshed at least once. */
  total: number | null;
  change: number | null;
  /** Merged by asset across the accounts, most valuable first. */
  assets: HeldAsset[];
  /** Largest unrealized PnL (either way) first. */
  positions: HeldPosition[];
}

export const MAX_INSTRUMENTS = 30;

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  /** The language the app speaks now. */
  getLocale: () => invoke<Locale>("get_locale"),
  getLoginItem: () => invoke<boolean>("get_login_item"),
  setLoginItem: (enabled: boolean) => invoke<boolean>("set_login_item", { enabled }),
  getUpdate: () => invoke<Update>("get_update"),
  /** Starts a check; what it finds arrives through `onUpdate`. */
  checkUpdate: () => invoke<void>("check_update"),
  restartToUpdate: () => invoke<void>("restart_to_update"),
  /** Tells the app the page has content, so its hidden window can be shown. */
  ready: () => invoke<void>("window_ready"),
  /** Instruments matching the query; an empty query only starts loading the lists. */
  search: (query: string) => invoke<Search>("search_instruments", { query }),
  /** The instrument the chart window should show (kept by the app, see onChartInstrument). */
  getChartInstrument: () => invoke<string | null>("get_chart_instrument"),
  openChart: (id: string) => invoke<void>("open_chart", { id }),
  chartSpec: (id: string) => invoke<ChartSpec>("chart_spec", { id }),
  /** An instrument outside the watchlist (from the synced Tonghuashun watchlist). */
  chartInstrument: (id: string) => invoke<Instrument>("chart_instrument", { id }),
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
  getLongbridge: () => invoke<Longbridge>("get_longbridge"),
  /** Saves the credentials, or removes them with null. */
  setLongbridge: (keys: LongbridgeKeys | null) => invoke<Longbridge>("set_longbridge", { keys }),
  /** Logs in with the saved credentials and reports what the account may see. */
  checkLongbridge: () => invoke<LongbridgeAccount>("check_longbridge"),
  getThs: () => invoke<Ths>("get_ths"),
  /** Checks with Tonghuashun that the key works and saves it; null removes it. */
  setThs: (key: string | null) => invoke<Ths>("set_ths", { key }),
  /** Opens Tonghuashun's sign-in page; signed in, `onThs` fires. */
  thsSignIn: () => invoke<void>("ths_sign_in"),
  thsSignOut: () => invoke<Ths>("ths_sign_out"),
  /** The Tonghuashun login or its watchlist changed: ask `getThs` again. */
  onThs: (handler: () => void): Promise<UnlistenFn> => listen("ths", () => handler()),
  getCoinGecko: () => invoke<CoinGecko>("get_coingecko"),
  /** Checks with CoinGecko what the key is and saves it; null removes it. */
  setCoinGecko: (key: string | null) => invoke<CoinGecko>("set_coingecko", { key }),
  /** Asks CoinGecko again what the saved key is; a paid plan's usage moves. */
  checkCoinGecko: () => invoke<CoinGeckoAccess>("check_coingecko"),
  getAgent: () => invoke<AgentStatus>("get_agent"),
  onAgent: (handler: (status: AgentStatus) => void): Promise<UnlistenFn> =>
    listen<AgentStatus>("agent", (event) => handler(event.payload)),
  getExchangeKeys: () => invoke<ExchangeKey[]>("get_exchange_keys"),
  /** Checks with its exchange that the key works and saves it; null removes it. */
  setExchangeKey: (exchange: Exchange, key: ApiKey | null) =>
    invoke<ExchangeKey[]>("set_exchange_key", { exchange, key }),
  getPortfolio: () => invoke<Portfolio>("get_portfolio"),
  openHoldings: () => invoke<void>("open_holdings"),
  openSettings: () => invoke<void>("open_settings"),
  /** Fresh holdings, while the holdings window is open: every refresh, and every price tick in between. */
  onPortfolio: (handler: (portfolio: Portfolio) => void): Promise<UnlistenFn> =>
    listen<Portfolio>("portfolio", (event) => handler(event.payload)),
  /** The language the app speaks from now on, whenever the settings change it. */
  onLocale: (handler: (locale: Locale) => void): Promise<UnlistenFn> =>
    listen<Locale>("locale", (event) => handler(event.payload)),
  onUpdate: (handler: (update: Update) => void): Promise<UnlistenFn> =>
    listen<Update>("update", (event) => handler(event.payload)),
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
