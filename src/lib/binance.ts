// Public Binance spot market data, fetched straight from the webview (WebKit
// honors the macOS proxy settings on its own).

import type { Coin } from "./api";

// The second host of each pair serves the same public market data and is
// reachable from some networks where the first is not.
export const REST_HOSTS = ["https://api.binance.com", "https://data-api.binance.vision"];
export const WS_HOSTS = ["wss://stream.binance.com:9443", "wss://data-stream.binance.vision"];

/** GET a public endpoint, trying each host in turn. */
export async function binanceGet<T>(path: string, signal?: AbortSignal): Promise<T> {
  let lastError: unknown = null;
  for (const host of REST_HOSTS) {
    try {
      const timeout = AbortSignal.timeout(10_000);
      const response = await fetch(host + path, {
        signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return (await response.json()) as T;
    } catch (error) {
      if (signal?.aborted) throw error;
      lastError = error;
    }
  }
  throw lastError;
}

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

const EXCHANGE_INFO = "/api/v3/exchangeInfo?permissions=SPOT&symbolStatus=TRADING&showPermissionSets=false";

let pairs: Promise<Pair[]> | null = null;

/** Fetched once per window; a failure is retried on the next call. */
export function loadPairs(): Promise<Pair[]> {
  pairs ??= binanceGet<{ symbols: RawSymbol[] }>(EXCHANGE_INFO)
    .then((body) =>
      body.symbols.map((s) => ({
        symbol: s.symbol,
        base: s.baseAsset,
        quote: s.quoteAsset,
        decimals: tickDecimals(s.filters.find((f) => f.filterType === "PRICE_FILTER")?.tickSize),
      })),
    )
    .catch((error: unknown) => {
      pairs = null;
      throw error;
    });
  return pairs;
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
