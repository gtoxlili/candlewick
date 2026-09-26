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
  /** Tells the app the page has painted, so the hidden window can be shown. */
  ready: () => invoke<void>("settings_ready"),
  onStatus: (handler: (status: Status) => void): Promise<UnlistenFn> =>
    listen<Status>("status", (event) => handler(event.payload)),
};

/** A tradable Binance spot pair. */
export interface Pair {
  symbol: string;
  base: string;
  quote: string;
  decimals: number;
}

interface RawSymbol {
  symbol: string;
  baseAsset: string;
  quoteAsset: string;
  filters: { filterType: string; tickSize?: string }[];
}

// The second host serves the same public market data and is reachable from
// some networks where api.binance.com is not.
const REST_HOSTS = ["https://api.binance.com", "https://data-api.binance.vision"];
const EXCHANGE_INFO = "/api/v3/exchangeInfo?permissions=SPOT&symbolStatus=TRADING&showPermissionSets=false";

let pairs: Promise<Pair[]> | null = null;

/** Fetched once per window; a failure is retried on the next call. */
export function loadPairs(): Promise<Pair[]> {
  pairs ??= fetchPairs().catch((error: unknown) => {
    pairs = null;
    throw error;
  });
  return pairs;
}

async function fetchPairs(): Promise<Pair[]> {
  let lastError: unknown = null;
  for (const host of REST_HOSTS) {
    try {
      const response = await fetch(host + EXCHANGE_INFO, { signal: AbortSignal.timeout(10_000) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const body = (await response.json()) as { symbols: RawSymbol[] };
      return body.symbols.map((s) => ({
        symbol: s.symbol,
        base: s.baseAsset,
        quote: s.quoteAsset,
        decimals: tickDecimals(s.filters.find((f) => f.filterType === "PRICE_FILTER")?.tickSize),
      }));
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError;
}

/** "0.01000000" → 2, "1.00000000" → 0 */
function tickDecimals(tickSize: string | undefined): number {
  const fraction = tickSize?.split(".")[1] ?? "";
  return fraction.replace(/0+$/, "").length;
}

const QUOTE_RANK = ["USDT", "USDC", "FDUSD", "BTC", "ETH", "BNB"];

export function searchPairs(all: Pair[], query: string, limit = 8): Pair[] {
  const q = normalize(query);
  if (!q) return [];
  const ranked: { pair: Pair; rank: number }[] = [];
  for (const pair of all) {
    let match: number;
    if (pair.base === q || pair.symbol === q) match = 0;
    else if (pair.base.startsWith(q)) match = 1;
    else if (pair.symbol.startsWith(q)) match = 2;
    else continue;
    const quote = QUOTE_RANK.indexOf(pair.quote);
    ranked.push({ pair, rank: match * 100 + (quote === -1 ? QUOTE_RANK.length : quote) });
  }
  ranked.sort((a, b) => a.rank - b.rank || a.pair.symbol.localeCompare(b.pair.symbol));
  return ranked.slice(0, limit).map((r) => r.pair);
}

const MANUAL_QUOTES = ["FDUSD", "USDT", "USDC", "USD1", "TUSD", "BTC", "ETH", "BNB", "EUR", "TRY"];

/**
 * Fallback when the pair list can't be fetched: "SOL/USDT" or "SOLUSDT".
 * Decimals stay unknown, so the app picks them by magnitude.
 */
export function parsePair(input: string): Coin | null {
  const raw = input.trim().toUpperCase();
  const [base, quote] = raw.includes("/")
    ? raw.split("/").map(normalize)
    : (() => {
        const symbol = normalize(raw);
        const q = MANUAL_QUOTES.find((q) => symbol.endsWith(q) && symbol.length > q.length);
        return q ? [symbol.slice(0, -q.length), q] : [];
      })();
  // Same rule the app enforces: letters and digits only (any script).
  const valid = (s: string | undefined): s is string => !!s && /^[\p{L}\p{N}]+$/u.test(s);
  if (!valid(base) || !valid(quote)) return null;
  return { symbol: base + quote, base, quote, decimals: null, pinned: false };
}

function normalize(s: string): string {
  return s.trim().toUpperCase().replace(/[\s/_-]/g, "");
}
