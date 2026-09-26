// Live market data for one pair: REST history for the visible time window,
// plus one websocket carrying 1-second bars, the top of the order book, the
// 24h ticker and (only while the trades tab is shown) aggregate trades.

import type { CandlePoint } from "liveline";

import { binanceGet, WS_HOSTS } from "@/lib/binance";

export type ChartMode = "line" | "candle";

export interface TimeWindow {
  label: string;
  /** Visible span, seconds. */
  secs: number;
  /** Seconds per candle: about 50–60 candles across the window. */
  candle: number;
  /** Binance bars fetched as history and grouped into candles. */
  source: "1s" | "1m" | "5m";
  limit: number;
  /** Short windows read best as a live line, long ones as candles. */
  mode: ChartMode;
}

export const WINDOWS = [
  { label: "5分钟", secs: 300, candle: 5, source: "1s", limit: 400, mode: "line" },
  { label: "15分钟", secs: 900, candle: 15, source: "1s", limit: 1000, mode: "line" },
  { label: "1小时", secs: 3600, candle: 60, source: "1m", limit: 75, mode: "candle" },
  { label: "4小时", secs: 14_400, candle: 300, source: "1m", limit: 260, mode: "candle" },
  { label: "1天", secs: 86_400, candle: 1800, source: "5m", limit: 300, mode: "candle" },
] as const satisfies readonly TimeWindow[];

export const windowBySecs = (secs: number): TimeWindow =>
  WINDOWS.find((w) => w.secs === secs) ?? WINDOWS[2];

/**
 * Everything the chart draws; replaced (never mutated) on each update. The
 * line is derived from the candles too, so either view stays ~60 points.
 */
export interface ChartData {
  candles: CandlePoint[];
  live: CandlePoint | null;
}

export interface Ticker {
  last: number;
  high: number;
  low: number;
  /** Base-asset volume over 24h. */
  volume: number;
  /** Quote-asset turnover over 24h. */
  quoteVolume: number;
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
  id: number;
  price: number;
  qty: number;
  /** Epoch milliseconds. */
  time: number;
  /** The buyer was the maker, so the taker sold. */
  sell: boolean;
}

export type FeedState = "connecting" | "live" | "offline";

export interface MarketEvents {
  /** History for the window arrived, or a live 1-second bar moved it on. */
  chart(data: ChartData): void;
  ticker(ticker: Ticker): void;
  book(book: Book): void;
  /** Newest first. */
  trades(list: Trade[]): void;
  state(state: FeedState): void;
}

const MAX_TRADES = 60;
/**
 * 1-second bars and the depth stream push every second, so this much silence
 * means a dead socket (e.g. after a network change). Generous enough that a
 * quiet, illiquid pair doesn't reconnect in a loop.
 */
const SILENCE_MS = 30_000;
const TRADES_FLUSH_MS = 250;
const MAX_CANDLES = 400;

export class Market {
  private readonly lower: string;
  private readonly query: string;
  private ws: WebSocket | null = null;
  private disposed = false;
  private suspended = false;
  private failures = 0;
  private host = 0;
  private requestId = 0;
  private retryTimer: number | undefined;
  private silenceTimer: number | undefined;
  private flushTimer: number | undefined;
  private rest = new AbortController();
  /** The history request in flight, if any. */
  private historyLoad: AbortController | null = null;
  private window: TimeWindow;
  /** Live bars are dropped until the history for the window is in. */
  private chartReady = false;
  private data: ChartData = { candles: [], live: null };
  private tradesOn: boolean;
  private tradeBuffer: Trade[] = [];

  constructor(
    symbol: string,
    window: TimeWindow,
    trades: boolean,
    private readonly on: MarketEvents,
  ) {
    this.lower = symbol.toLowerCase();
    this.query = encodeURIComponent(symbol);
    this.window = window;
    this.tradesOn = trades;
  }

  start(): void {
    this.loadSnapshot();
    this.connect();
  }

  dispose(): void {
    this.disposed = true;
    this.rest.abort();
    this.historyLoad?.abort();
    this.clearTimers();
    this.drop();
  }

  /** New span: fetch its history; the live stream stays as it is. */
  setWindow(window: TimeWindow): void {
    if (window.secs === this.window.secs) return;
    this.window = window;
    this.chartReady = false;
    this.loadHistory();
  }

  setTrades(on: boolean): void {
    if (on === this.tradesOn) return;
    this.tradesOn = on;
    const stream = `${this.lower}@aggTrade`;
    if (on) {
      this.resubscribe([], [stream]);
      this.loadTrades();
    } else {
      this.resubscribe([stream], []);
      this.tradeBuffer = [];
    }
  }

  /** Nobody is looking: close the socket until `resume`. */
  suspend(): void {
    if (this.suspended || this.disposed) return;
    this.suspended = true;
    this.clearTimers();
    this.drop();
    this.on.state("offline");
  }

  /** Catch up on what was missed, then stream again. */
  resume(): void {
    if (!this.suspended || this.disposed) return;
    this.suspended = false;
    this.failures = 0;
    this.chartReady = false;
    this.loadSnapshot();
    this.connect();
  }

  private streams(): string[] {
    const list = [`${this.lower}@kline_1s`, `${this.lower}@depth20`, `${this.lower}@ticker`];
    if (this.tradesOn) list.push(`${this.lower}@aggTrade`);
    return list;
  }

  private connect(): void {
    if (this.disposed || this.suspended) return;
    this.on.state("connecting");
    // Symbols may be non-ASCII; "@" can stay as is.
    const param = (s: string) => encodeURIComponent(s).replaceAll("%40", "@");
    const base = WS_HOSTS[this.host % WS_HOSTS.length];
    const ws = new WebSocket(`${base}/stream?streams=${this.streams().map(param).join("/")}`);
    this.ws = ws;
    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.failures = 0;
      this.on.state("live");
      this.armSilence();
      // A history request that failed earlier gets another go now.
      if (!this.chartReady && !this.historyLoad) this.loadHistory();
    };
    ws.onmessage = (event: MessageEvent<string>) => {
      if (this.ws !== ws) return;
      this.armSilence();
      this.handle(event.data);
    };
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      this.reconnectLater();
    };
  }

  /** Forgets the current socket without waiting for its close handshake. */
  private drop(): void {
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      ws.onopen = ws.onmessage = ws.onclose = null;
      ws.close();
    }
  }

  private reconnectLater(): void {
    if (this.disposed || this.suspended) return;
    this.clearTimers();
    this.failures += 1;
    this.host += 1;
    this.chartReady = false;
    this.on.state("offline");
    const delay = Math.min(1000 * 2 ** Math.min(this.failures - 1, 5), 30_000);
    this.retryTimer = window.setTimeout(() => {
      this.loadSnapshot();
      this.connect();
    }, delay);
  }

  private armSilence(): void {
    window.clearTimeout(this.silenceTimer);
    this.silenceTimer = window.setTimeout(() => {
      this.drop();
      this.reconnectLater();
    }, SILENCE_MS);
  }

  private clearTimers(): void {
    window.clearTimeout(this.retryTimer);
    window.clearTimeout(this.silenceTimer);
    window.clearTimeout(this.flushTimer);
    this.flushTimer = undefined;
  }

  private resubscribe(remove: string[], add: string[]): void {
    const ws = this.ws;
    if (ws?.readyState === WebSocket.OPEN) {
      if (remove.length) ws.send(JSON.stringify({ method: "UNSUBSCRIBE", params: remove, id: ++this.requestId }));
      if (add.length) ws.send(JSON.stringify({ method: "SUBSCRIBE", params: add, id: ++this.requestId }));
    } else if (ws) {
      // Still connecting with the old stream list: start over with the new one.
      this.drop();
      this.connect();
    }
  }

  private handle(raw: string): void {
    const { stream, data } = JSON.parse(raw) as { stream?: string; data?: unknown };
    if (!stream || !data) return; // subscription acknowledgements
    if (stream === `${this.lower}@kline_1s`) {
      if (this.chartReady) this.tick(data as RawWsKline);
    } else if (stream === `${this.lower}@depth20`) {
      this.on.book(parseBook(data as RawBook));
    } else if (stream === `${this.lower}@ticker`) {
      this.on.ticker(parseWsTicker(data as RawWsTicker));
    } else if (stream === `${this.lower}@aggTrade` && this.tradesOn) {
      this.pushTrades([parseTrade(data as RawTrade)]);
    }
  }

  /** One live second: fold the bar into the forming candle. */
  private tick({ k }: RawWsKline): void {
    const close = +k.c;
    const { candles, live } = this.data;
    const bucket = Math.floor(k.t / 1000 / this.window.candle) * this.window.candle;
    let nextCandles = candles;
    let nextLive: CandlePoint;
    if (!live || bucket > live.time) {
      if (live) nextCandles = [...candles, live].slice(-MAX_CANDLES);
      nextLive = { time: bucket, open: +k.o, high: +k.h, low: +k.l, close };
    } else {
      nextLive = { ...live, high: Math.max(live.high, +k.h), low: Math.min(live.low, +k.l), close };
    }

    this.data = { candles: nextCandles, live: nextLive };
    this.on.chart(this.data);
  }

  private pushTrades(trades: Trade[]): void {
    // Newest first; the trade id orders them even within one millisecond.
    this.tradeBuffer = [...trades, ...this.tradeBuffer]
      .sort((a, b) => b.id - a.id)
      .filter((t, i, all) => i === 0 || t.id !== all[i - 1].id)
      .slice(0, MAX_TRADES);
    if (this.flushTimer !== undefined) return;
    this.flushTimer = window.setTimeout(() => {
      this.flushTimer = undefined;
      this.on.trades(this.tradeBuffer);
    }, TRADES_FLUSH_MS);
  }

  private loadSnapshot(): void {
    this.loadHistory();
    const signal = this.rest.signal;
    binanceGet<RawRestTicker>(`/api/v3/ticker/24hr?symbol=${this.query}`, signal)
      .then((t) => this.on.ticker(parseRestTicker(t)))
      .catch(() => {});
    binanceGet<RawBook>(`/api/v3/depth?symbol=${this.query}&limit=20`, signal)
      .then((b) => this.on.book(parseBook(b)))
      .catch(() => {});
    if (this.tradesOn) this.loadTrades();
  }

  private loadHistory(): void {
    this.historyLoad?.abort();
    const load = new AbortController();
    this.historyLoad = load;
    const window = this.window;
    binanceGet<RawRestKline[]>(
      `/api/v3/klines?symbol=${this.query}&interval=${window.source}&limit=${window.limit}`,
      AbortSignal.any([this.rest.signal, load.signal]),
    )
      .then((rows) => {
        if (load.signal.aborted || window !== this.window) return;
        this.data = fromBars(rows, window.candle);
        this.chartReady = true;
        this.on.chart(this.data);
      })
      // Retried when the socket (re)connects.
      .catch(() => {})
      .finally(() => {
        if (this.historyLoad === load) this.historyLoad = null;
      });
  }

  private loadTrades(): void {
    binanceGet<RawTrade[]>(`/api/v3/aggTrades?symbol=${this.query}&limit=40`, this.rest.signal)
      .then((list) => {
        if (this.tradesOn) this.pushTrades(list.map(parseTrade));
      })
      .catch(() => {});
  }
}

/** Candles from bars grouped by `candle` seconds. */
function fromBars(rows: RawRestKline[], candle: number): ChartData {
  const candles: CandlePoint[] = [];
  for (const [openMs, o, h, l, c] of rows) {
    const close = +c;
    const bucket = Math.floor(openMs / 1000 / candle) * candle;
    const forming = candles.at(-1);
    if (forming && bucket <= forming.time) {
      candles[candles.length - 1] = {
        ...forming,
        high: Math.max(forming.high, +h),
        low: Math.min(forming.low, +l),
        close,
      };
    } else {
      candles.push({ time: bucket, open: +o, high: +h, low: +l, close });
    }
  }
  // The newest bucket is still forming.
  const live = candles.pop() ?? null;
  return { candles, live };
}

type RawRestKline = [number, string, string, string, string, ...unknown[]];

interface RawWsKline {
  k: { t: number; o: string; h: string; l: string; c: string };
}

interface RawBook {
  bids: [string, string][];
  asks: [string, string][];
}

interface RawWsTicker {
  c: string;
  h: string;
  l: string;
  v: string;
  q: string;
  p: string;
  P: string;
}

interface RawRestTicker {
  lastPrice: string;
  highPrice: string;
  lowPrice: string;
  volume: string;
  quoteVolume: string;
  priceChange: string;
  priceChangePercent: string;
}

interface RawTrade {
  a: number;
  p: string;
  q: string;
  T: number;
  m: boolean;
}

function parseBook(b: RawBook): Book {
  const levels = (side: [string, string][]) => side.map(([p, q]) => ({ price: +p, qty: +q }));
  return { bids: levels(b.bids), asks: levels(b.asks) };
}

function parseWsTicker(t: RawWsTicker): Ticker {
  return {
    last: +t.c,
    high: +t.h,
    low: +t.l,
    volume: +t.v,
    quoteVolume: +t.q,
    change: +t.p,
    changePct: +t.P,
  };
}

function parseRestTicker(t: RawRestTicker): Ticker {
  return {
    last: +t.lastPrice,
    high: +t.highPrice,
    low: +t.lowPrice,
    volume: +t.volume,
    quoteVolume: +t.quoteVolume,
    change: +t.priceChange,
    changePct: +t.priceChangePercent,
  };
}

function parseTrade(t: RawTrade): Trade {
  return { id: t.a, price: +t.p, qty: +t.q, time: t.T, sell: t.m };
}
