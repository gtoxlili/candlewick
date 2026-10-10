// Runs the pages in a plain browser (`pnpm dev`, then open /holdings.html
// etc. in Chrome) with the app's IPC answered from fixtures, so a page can be
// iterated on without the app. Only the development build imports this, and
// only when Tauri's IPC is absent (see each page's main.tsx).

import type { Channel } from "@tauri-apps/api/core";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import type { Candle, ChartSpec, Instrument, LiveEvent, Portfolio, Search, Settings, Trade } from "@/lib/api";
import { LOCALES, type Locale } from "@/lib/i18n";

const pool: Instrument = {
  provider: "dexscreener",
  symbol: "solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx",
  base: "$WIF",
  quote: "USD",
  name: "dogwifhat",
  decimals: null,
  pinned: false,
  venue: { chain: "Solana", dex: "Raydium" },
};

const settings: Settings = {
  exchange: "binance",
  watchlist: [
    { provider: "binance", symbol: "BTCUSDT", base: "BTC", quote: "USDT", decimals: 2, pinned: true },
    { provider: "binance", symbol: "ETHUSDT", base: "ETH", quote: "USDT", decimals: 2, pinned: false },
    pool,
  ],
  showSymbol: true,
  showChange: false,
  holdingsInBar: false,
  colorScheme: "greenUp",
  autoUpdate: true,
  agentAccess: false,
  language: "system",
};

const now = Date.now();

/** Two exchanges, a few assets on both, two futures positions. */
const portfolio: Portfolio = {
  accounts: [
    {
      exchange: "binance",
      total: 78_210.1,
      change: 712.3,
      wallets: [
        { wallet: "spot", value: 58_120.4 },
        { wallet: "earn", value: 12_000 },
        { wallet: "usdFutures", value: 8_089.7 },
      ],
      updated: now - 4_000,
      error: null,
    },
    {
      exchange: "okx",
      total: 50_220.42,
      change: 860,
      wallets: [
        { wallet: "trading", value: 48_020.42 },
        { wallet: "funding", value: 2_200 },
      ],
      updated: now - 9_000,
      error: null,
    },
  ],
  total: 128_430.52,
  change: 1_572.3,
  assets: [
    {
      asset: "BTC",
      amount: 0.7612,
      price: 84_100.77,
      value: 64_017.5,
      change: 1_320.2,
      changePct: 2.1,
      stable: false,
      held: [
        { exchange: "binance", wallet: "spot", amount: 0.5 },
        { exchange: "okx", wallet: "trading", amount: 0.2612 },
      ],
      cost: 61_200,
      pnl: 5_980.2,
    },
    {
      asset: "ETH",
      amount: 6.4,
      price: 3_412.1,
      value: 21_837.4,
      change: -210.6,
      changePct: -0.95,
      stable: false,
      held: [
        { exchange: "binance", wallet: "spot", amount: 4 },
        { exchange: "binance", wallet: "earn", amount: 2.4 },
      ],
      cost: null,
      pnl: null,
    },
    {
      asset: "USDT",
      amount: 30_890.2,
      price: 1,
      value: 30_890.2,
      change: 0,
      changePct: 0,
      stable: true,
      held: [
        { exchange: "okx", wallet: "trading", amount: 20_010.2 },
        { exchange: "binance", wallet: "usdFutures", amount: 8_680 },
        { exchange: "okx", wallet: "funding", amount: 2_200 },
      ],
      cost: null,
      pnl: null,
    },
    {
      asset: "SOL",
      amount: 42.5,
      price: 178.4,
      value: 7_582,
      change: 310.4,
      changePct: 4.27,
      stable: false,
      held: [{ exchange: "okx", wallet: "trading", amount: 42.5 }],
      cost: 150.1,
      pnl: 1_202.7,
    },
    {
      asset: "USDC",
      amount: 2_000,
      price: 1,
      value: 2_000,
      change: 0,
      changePct: 0,
      stable: true,
      held: [{ exchange: "binance", wallet: "spot", amount: 2_000 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "BNB",
      amount: 1.8,
      price: 612.3,
      value: 1_102.1,
      change: 12.4,
      changePct: 1.14,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 1.8 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "PEPE",
      amount: 52_000_000,
      price: 0.00001234,
      value: 641.7,
      change: 140.1,
      changePct: 27.9,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 52_000_000 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "ARB",
      amount: 310,
      price: 1.17,
      value: 362.7,
      change: -8.2,
      changePct: -2.2,
      stable: false,
      held: [{ exchange: "okx", wallet: "trading", amount: 310 }],
      cost: 1.4,
      pnl: -71.3,
    },
    {
      asset: "DOGE",
      amount: 12,
      price: 0.21,
      value: 2.52,
      change: 0.04,
      changePct: 1.6,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 12 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "ATOM",
      amount: 0.03,
      price: 8.1,
      value: 0.24,
      change: 0,
      changePct: 0.3,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 0.03 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "NOPAIR",
      amount: 3,
      price: null,
      value: null,
      change: null,
      changePct: null,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 3 }],
      cost: null,
      pnl: null,
    },
  ],
  positions: [
    {
      exchange: "binance",
      symbol: "BTCUSDT",
      kind: "usdtPerpetual",
      long: true,
      size: 0.4,
      sizeUnit: "BTC",
      entry: 81_240,
      mark: 84_100.77,
      liquidation: 62_310,
      leverage: 5,
      isolated: false,
      pnl: 1_144.3,
      pnlAsset: "USDT",
      pnlUsd: 1_144.3,
      exposure: 33_640.3,
      change: 690.1,
    },
    {
      exchange: "okx",
      symbol: "ETH-USDT-SWAP",
      kind: "usdtPerpetual",
      long: false,
      size: 20,
      sizeUnit: null,
      entry: 3_380,
      mark: 3_412.1,
      liquidation: 3_610,
      leverage: 10,
      isolated: true,
      pnl: -64.2,
      pnlAsset: "USDT",
      pnlUsd: -64.2,
      exposure: -6_824.2,
      change: 65,
    },
  ],
};

/** A portfolio before any account has answered. */
const loading: Portfolio = {
  accounts: [{ exchange: "binance", total: null, change: null, wallets: [], updated: null, error: null }],
  total: null,
  change: null,
  assets: [],
  positions: [],
};

// --- The chart window: a deterministic random walk, streamed a tick a second. ---

const INTERVALS = [60, 300, 900, 3600, 14_400, 86_400];
/** Candles before this many intervals ago don't exist, so paging back ends. */
const HISTORY = 1500;

const isPool = (id: string) => id.startsWith("dexscreener:");

function spec(id: string): ChartSpec {
  const onchain = isPool(id);
  return {
    source: onchain ? "dexscreener" : "binance",
    intervals: INTERVALS.map((secs) => ({ secs, mode: "candle", aligned: true, regularOnly: false })),
    statsSpan: "rolling24h",
    volumeUnit: onchain ? "$WIF" : "BTC",
    turnoverUnit: onchain ? "USD" : "USDT",
    dayOffset: 0,
    bookSteps: [],
    book: !onchain,
    onchain,
    link: null,
  };
}

/** A hash in [-0.5, 0.5) of an integer, the same every time. */
function noise(k: number): number {
  let x = Math.imul(k, 2_654_435_761) >>> 0;
  x ^= x >>> 15;
  x = Math.imul(x, 2_246_822_519) >>> 0;
  x ^= x >>> 13;
  return x / 4_294_967_296 - 0.5;
}

/** The walk from its origin to `to`, so any page agrees with any other. */
function walk(id: string, interval: number, to: number, from: number): Candle[] {
  const base = isPool(id) ? 0.00001234 : 84_000;
  const origin = to - HISTORY * interval;
  let price = base;
  const out: Candle[] = [];
  for (let t = origin; t <= to; t += interval) {
    const k = t / interval;
    const open = price;
    // A step of noise, pulled gently back toward the base so the walk never wanders off.
    const close = open * Math.exp(noise(k) * 0.012 + 0.01 * Math.log(base / open));
    const high = Math.max(open, close) * (1 + Math.abs(noise(k + 7)) * 0.006);
    const low = Math.min(open, close) * (1 - Math.abs(noise(k + 13)) * 0.006);
    if (t >= from) out.push({ time: t, open, high, low, close });
    price = close;
  }
  return out;
}

const lastBucket = (interval: number) => Math.floor(Date.now() / 1000 / interval) * interval;

function candles(id: string, interval: number, end: number | null, limit: number): Candle[] {
  const last = lastBucket(interval);
  const to = end === null ? last : Math.floor((end - 1) / interval) * interval;
  if (to < last - HISTORY * interval) return [];
  const from = Math.max(last - HISTORY * interval, to - (limit - 1) * interval);
  // The walk runs from the live edge's origin so the latest page and older pages share one history.
  return walk(id, interval, last, from).filter((c) => c.time <= to);
}

let nextTrade = 1;

function trade(id: string, price: number, time: number): Trade {
  return {
    id: nextTrade++,
    price,
    qty: isPool(id) ? 50 + Math.random() * 2000 : Math.random() * 0.5,
    time,
    sell: Math.random() < 0.5,
    extended: false,
  };
}

const streams = new Map<number, number>();
let nextStream = 1;

function stream(id: string, events: Channel<LiveEvent>): number {
  const handle = nextStream++;
  const onchain = isPool(id);
  let price = candles(id, 60, null, 1)[0]?.close ?? 1;
  const send = (event: LiveEvent) => events.onmessage(event);
  const stats = () =>
    send({
      kind: "stats",
      stats: {
        last: price,
        high: price * 1.031,
        low: price * 0.974,
        volume: onchain ? null : 12_345.6,
        turnover: onchain ? 289_043.58 : 1.04e9,
        change: price - price / 1.0288,
        changePct: 2.88,
        onchain: onchain
          ? { liquidity: 6_141_965.07, marketCap: 217_289_257, fdv: 217_289_257, buys: 743, sells: 684 }
          : null,
      },
    });
  send({ kind: "state", state: "live" });
  stats();
  const timer = window.setInterval(() => {
    price *= Math.exp((Math.random() - 0.5) * 0.002);
    send({ kind: "trades", trades: [trade(id, price, Date.now())] });
    stats();
    if (!onchain) {
      const level = (i: number, side: 1 | -1) => ({ price: price * (1 + side * 0.0001 * (i + 1)), qty: Math.random() * 3 });
      send({
        kind: "book",
        books: [{ bids: Array.from({ length: 20 }, (_, i) => level(i, -1)), asks: Array.from({ length: 20 }, (_, i) => level(i, 1)) }],
      });
    }
  }, 700);
  streams.set(handle, timer);
  return handle;
}

function search(query: string): Search {
  const q = query.trim().toUpperCase();
  if (!q) return { candidates: [], notes: [] };
  const candidates: Search["candidates"] = [];
  if ("SOL".startsWith(q)) {
    candidates.push({ provider: "binance", symbol: "SOLUSDT", base: "SOL", quote: "USDT", decimals: 2, pinned: false, manual: false, liquidity: null });
  }
  if ("WIF".startsWith(q) || "$WIF".startsWith(q)) {
    candidates.push({ ...pool, manual: false, liquidity: 6_141_965.07 });
    candidates.push({
      ...pool,
      symbol: "solana/8sLbNZoA1cfnvMJLPfp98ZLAnFSYCFApfJKMbiXNLwxj",
      venue: { chain: "Solana", dex: "Orca" },
      manual: false,
      liquidity: 158_988.96,
    });
  }
  return { candidates, notes: [] };
}

/**
 * Answers the app's commands for `label`'s page. `?state=empty|loading` picks
 * a portfolio; `?lang=en|zh-CN|ja` the language the app speaks (default: the
 * first of the browser's that it has). The chart page draws a synthetic
 * market for whichever id its URL names.
 */
export function installMocks(label: string): void {
  mockWindows(label);
  const params = new URLSearchParams(location.search);
  const state = params.get("state");
  const locales = Object.keys(LOCALES) as Locale[];
  const speaks = (tag: string) => locales.find((l) => l === tag) ?? locales.find((l) => l.split("-")[0] === tag.split("-")[0]);
  const locale = [params.get("lang") ?? "", ...navigator.languages].map(speaks).find((l) => l !== undefined) ?? "en";
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "get_settings":
        return settings;
      case "get_locale":
        return locale;
      case "get_portfolio":
        return state === "empty" ? { ...loading, accounts: [] } : state === "loading" ? loading : portfolio;
      case "search_instruments":
        return search(String(args.query ?? ""));
      case "chart_spec":
        return spec(String(args.id));
      case "chart_history":
        return candles(String(args.id), Number(args.interval), args.end === null ? null : Number(args.end), Number(args.limit));
      case "chart_trades": {
        const id = String(args.id);
        const price = candles(id, 60, null, 1)[0]?.close ?? 1;
        return Array.from({ length: Number(args.limit) }, (_, i) =>
          trade(id, price * (1 + (Math.random() - 0.5) * 0.001), Date.now() - (Number(args.limit) - i) * 1500),
        );
      }
      case "chart_stream":
        return stream(String(args.id), args.events as Channel<LiveEvent>);
      case "chart_stream_stop":
        window.clearInterval(streams.get(Number(args.handle)));
        streams.delete(Number(args.handle));
        return null;
      case "plugin:event|listen":
        return 1;
      default:
        return null;
    }
  });
}
