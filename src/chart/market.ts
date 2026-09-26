// Live market data for one pair: candle history per interval over REST, plus
// one websocket carrying every trade, the top of the order book and the 24h
// ticker. Each trade moves the forming candle and the newest stretch of the
// line, so the chart moves with the market rather than once a second.

import type { CandlePoint, LivelinePoint } from "liveline";

import { binanceGet, WS_HOSTS } from "@/lib/binance";

export type ChartMode = "line" | "candle";

export interface Interval {
  label: string;
  /** Seconds per candle. */
  secs: number;
  /** Binance's name for the interval. */
  api: string;
  /** How it opens: one-second candles are mostly noise, a line reads better. */
  mode: ChartMode;
}

export const INTERVALS = [
  { label: "1秒", secs: 1, api: "1s", mode: "line" },
  { label: "1分", secs: 60, api: "1m", mode: "candle" },
  { label: "5分", secs: 300, api: "5m", mode: "candle" },
  { label: "15分", secs: 900, api: "15m", mode: "candle" },
  { label: "1小时", secs: 3600, api: "1h", mode: "candle" },
  { label: "4小时", secs: 14_400, api: "4h", mode: "candle" },
  { label: "1日", secs: 86_400, api: "1d", mode: "candle" },
] as const satisfies readonly Interval[];

export const intervalBySecs = (secs: number): Interval =>
  INTERVALS.find((i) => i.secs === secs) ?? INTERVALS[1];

/** What the chart draws for the selected interval; a new object on every change. */
export interface ChartData {
  interval: number;
  /** Closed candles, oldest first. */
  candles: CandlePoint[];
  /** The forming candle. */
  live: CandlePoint | null;
  /** Candle closes, then trade-by-trade detail from since the window opened. */
  line: LivelinePoint[];
  /** Latest trade price. */
  price: number | null;
  /** Nothing older can be paged in. */
  exhausted: boolean;
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
  ticker(ticker: Ticker): void;
  book(book: Book): void;
  /** Newest first. */
  trades(list: Trade[]): void;
  state(state: FeedState): void;
}

/** Binance's most klines per request. */
const PAGE = 1000;
/** Bounds memory and per-frame work however far back someone scrolls. */
const MAX_CANDLES = 6000;
/** Line detail kept: ten minutes of one-second candles at four points a second. */
const MAX_TRAIL = 2400;
/** Trades held while an interval's history loads. */
const MAX_PENDING = 5000;
/** Trades arrive dozens a second; the chart eases between these updates. */
const CHART_FLUSH_MS = 100;
const TRADES_FLUSH_MS = 250;
const MAX_TRADES = 60;
const HISTORY_RETRY_MS = 5000;
/** Paging back retries after this, doubling per failure: a 429 left unanswered gets the IP banned. */
const OLDER_RETRY_MS = 2000;
const OLDER_RETRY_MAX_MS = 60_000;
/**
 * The depth and ticker streams push every second, so this much silence means
 * a dead socket (e.g. after a network change).
 */
const SILENCE_MS = 30_000;

/** History and live state of one interval. */
class Series {
  candles: CandlePoint[] = [];
  live: CandlePoint | null = null;
  /** Trade detail for the line, one point per `step` seconds. */
  trail: LivelinePoint[] = [];
  /** Candle closes before the trail starts; rebuilt after history changes. */
  private history: LivelinePoint[] | null = null;
  /** History is current, so trades apply directly instead of waiting in `pending`. */
  ready = false;
  pending: { price: number; time: number }[] = [];
  exhausted = false;
  load: AbortController | null = null;
  older: AbortController | null = null;
  olderBackoff = 0;
  olderRetryAt = 0;

  constructor(readonly interval: Interval) {}

  /** A point per sixtieth of a candle, at most four a second. */
  private get step(): number {
    return Math.max(this.interval.secs / 60, 0.25);
  }

  /** One trade: fold it into the forming candle and the line's detail. */
  add(price: number, time: number): void {
    const secs = this.interval.secs;
    const bucket = Math.floor(time / secs) * secs;
    const live = this.live;
    if (!live || bucket > live.time) {
      if (live) this.close(live);
      this.live = { time: bucket, open: price, high: price, low: price, close: price };
    } else if (bucket === live.time) {
      this.live = {
        time: bucket,
        open: live.open,
        high: Math.max(live.high, price),
        low: Math.min(live.low, price),
        close: price,
      };
    } else {
      return; // part of a candle the history already has
    }

    const last = this.trail.at(-1);
    if (last && Math.floor(last.time / this.step) === Math.floor(time / this.step)) {
      this.trail[this.trail.length - 1] = { time: last.time, value: price };
    } else if (!last || time > last.time) {
      this.trail.push({ time, value: price });
      if (this.trail.length > MAX_TRAIL) {
        this.trail.splice(0, this.trail.length - MAX_TRAIL);
        this.history = null;
      }
    }
  }

  private close(candle: CandlePoint): void {
    const candles = [...this.candles, candle];
    if (candles.length > MAX_CANDLES) {
      this.candles = candles.slice(-MAX_CANDLES);
      this.exhausted = false;
    } else {
      this.candles = candles;
    }
    this.history = null;
  }

  /**
   * The newest page from REST. Pages already scrolled in stay only if they
   * join up with it: after a long gap they would leave a hole that paging back
   * never reaches. The trail always goes, or the line would bridge the gap
   * straight. Returns whether older pages were kept.
   */
  replaceRecent(fresh: CandlePoint[]): boolean {
    const live = fresh.pop() ?? null;
    const from = fresh[0]?.time ?? live?.time ?? Infinity;
    let kept = this.candles.filter((c) => c.time < from);
    const last = kept.at(-1);
    if (last && last.time + this.interval.secs < from) kept = [];
    this.candles = [...kept, ...fresh].slice(-MAX_CANDLES);
    this.live = live;
    this.trail = [];
    this.history = null;
    return kept.length > 0;
  }

  prepend(page: CandlePoint[]): void {
    const first = this.candles[0]?.time ?? Infinity;
    this.candles = [...page.filter((c) => c.time < first), ...this.candles];
    this.history = null;
  }

  /**
   * The line: each candle's close at its center (where the candle/line morph
   * puts it), then the trade detail. Where the detail exists it replaces the
   * closes, so the two never zigzag against each other.
   */
  line(): LivelinePoint[] {
    if (!this.history) {
      const cut = this.trail[0]?.time ?? Infinity;
      const half = this.interval.secs / 2;
      const points: LivelinePoint[] = [];
      for (const c of this.candles) {
        const time = c.time + half;
        if (time >= cut) break;
        points.push({ time, value: c.close });
      }
      this.history = points;
    }
    return this.trail.length ? this.history.concat(this.trail) : this.history;
  }
}

export class Market {
  private readonly lower: string;
  private readonly query: string;
  private ws: WebSocket | null = null;
  private disposed = false;
  private suspended = false;
  private failures = 0;
  private host = 0;
  private retryTimer: number | undefined;
  private silenceTimer: number | undefined;
  private historyTimer: number | undefined;
  private chartTimer: number | undefined;
  private tradesTimer: number | undefined;
  private rest = new AbortController();
  /** Every interval looked at so far: switching back is instant and animates. */
  private series = new Map<number, Series>();
  private current: Series;
  private price: number | null = null;
  private snapshot: ChartData | null = null;
  private listeners = new Set<() => void>();
  private tradesOn: boolean;
  private tradeList: Trade[] = [];

  constructor(
    symbol: string,
    interval: Interval,
    trades: boolean,
    private readonly on: MarketEvents,
  ) {
    this.lower = symbol.toLowerCase();
    this.query = encodeURIComponent(symbol);
    this.current = this.seriesFor(interval);
    this.tradesOn = trades;
  }

  start(): void {
    this.loadSnapshot();
    this.connect();
  }

  dispose(): void {
    this.disposed = true;
    this.rest.abort();
    this.clearTimers();
    this.drop();
    this.listeners.clear();
  }

  /** For `useSyncExternalStore`: the selected interval's chart data. */
  subscribeChart = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  chartData = (): ChartData | null => this.snapshot;

  selectInterval(interval: Interval): void {
    if (interval.secs === this.current.interval.secs) return;
    this.current = this.seriesFor(interval);
    if (!this.current.ready && !this.current.load) this.loadHistory(this.current);
    this.publish();
  }

  /** The view scrolled near the oldest candle: fetch the page before it. */
  loadOlder(): void {
    const s = this.current;
    const first = s.candles[0];
    // A page never takes more than is left under the cap, or the next closed
    // candle would trim most of it straight off again.
    const room = MAX_CANDLES - s.candles.length;
    if (!s.ready || s.exhausted || s.older || !first || room <= 0 || Date.now() < s.olderRetryAt) return;
    const limit = Math.min(PAGE, room);
    const older = new AbortController();
    s.older = older;
    binanceGet<RawRestKline[]>(
      `/api/v3/klines?symbol=${this.query}&interval=${s.interval.api}&endTime=${first.time * 1000 - 1}&limit=${limit}`,
      AbortSignal.any([this.rest.signal, older.signal]),
    )
      .then((rows) => {
        if (older.signal.aborted) return;
        s.prepend(rows.map(parseKline));
        s.olderBackoff = 0;
        if (rows.length < limit) s.exhausted = true;
        if (s === this.current) this.publish();
      })
      // Asked again on the next update near the edge, once the backoff has passed.
      .catch(() => {
        s.olderBackoff = Math.min(Math.max(s.olderBackoff * 2, OLDER_RETRY_MS), OLDER_RETRY_MAX_MS);
        s.olderRetryAt = Date.now() + s.olderBackoff;
      })
      .finally(() => {
        if (s.older === older) s.older = null;
      });
  }

  setTrades(on: boolean): void {
    if (on === this.tradesOn) return;
    this.tradesOn = on;
    if (!on) return;
    if (this.tradeList.length < 40) this.loadTrades();
    else this.on.trades(this.tradeList);
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
    this.loadSnapshot();
    this.connect();
  }

  private seriesFor(interval: Interval): Series {
    let s = this.series.get(interval.secs);
    if (!s) {
      s = new Series(interval);
      this.series.set(interval.secs, s);
    }
    return s;
  }

  private connect(): void {
    if (this.disposed || this.suspended) return;
    this.on.state("connecting");
    const streams = [`${this.lower}@aggTrade`, `${this.lower}@depth20`, `${this.lower}@ticker`];
    // Symbols may be non-ASCII; "@" can stay as is.
    const param = (s: string) => encodeURIComponent(s).replaceAll("%40", "@");
    const base = WS_HOSTS[this.host % WS_HOSTS.length];
    const ws = new WebSocket(`${base}/stream?streams=${streams.map(param).join("/")}`);
    this.ws = ws;
    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.failures = 0;
      this.on.state("live");
      this.armSilence();
      // A history request that failed earlier gets another go now.
      if (!this.current.ready && !this.current.load) this.loadHistory(this.current);
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
    for (const timer of [this.retryTimer, this.silenceTimer, this.historyTimer, this.chartTimer, this.tradesTimer]) {
      window.clearTimeout(timer);
    }
    this.historyTimer = this.chartTimer = this.tradesTimer = undefined;
  }

  private handle(raw: string): void {
    const { stream, data } = JSON.parse(raw) as { stream?: string; data?: unknown };
    if (!stream || !data) return;
    if (stream === `${this.lower}@aggTrade`) {
      this.trade(data as RawTrade);
    } else if (stream === `${this.lower}@depth20`) {
      this.on.book(parseBook(data as RawBook));
    } else if (stream === `${this.lower}@ticker`) {
      this.on.ticker(parseWsTicker(data as RawWsTicker));
    }
  }

  private trade(raw: RawTrade): void {
    const price = +raw.p;
    const time = raw.T / 1000;
    this.price = price;
    // Every cached interval stays live, not only the one on screen.
    for (const s of this.series.values()) {
      if (s.ready) {
        s.add(price, time);
      } else if (s.load) {
        s.pending.push({ price, time });
        if (s.pending.length > MAX_PENDING) s.pending = s.pending.slice(-MAX_PENDING / 2);
      }
    }
    this.pushTrades([parseTrade(raw)]);
    if (this.chartTimer === undefined) {
      this.chartTimer = window.setTimeout(() => {
        this.chartTimer = undefined;
        this.publish();
      }, CHART_FLUSH_MS);
    }
  }

  private publish(): void {
    const s = this.current;
    this.snapshot = {
      interval: s.interval.secs,
      candles: s.candles,
      live: s.live,
      line: s.line(),
      price: this.price ?? s.live?.close ?? null,
      exhausted: s.exhausted || s.candles.length >= MAX_CANDLES,
    };
    for (const listener of this.listeners) listener();
  }

  /** Newest first; the trade id orders them even within one millisecond. */
  private pushTrades(trades: Trade[]): void {
    this.tradeList = [...trades, ...this.tradeList]
      .sort((a, b) => b.id - a.id)
      .filter((t, i, all) => i === 0 || t.id !== all[i - 1].id)
      .slice(0, MAX_TRADES);
    if (!this.tradesOn || this.tradesTimer !== undefined) return;
    this.tradesTimer = window.setTimeout(() => {
      this.tradesTimer = undefined;
      if (this.tradesOn) this.on.trades(this.tradeList);
    }, TRADES_FLUSH_MS);
  }

  /** After a gap (start, reconnect, resume): fresh history, ticker and book. */
  private loadSnapshot(): void {
    // Until trades flow again the fresh history's close beats the last price seen.
    this.price = null;
    // Other intervals missed trades too; they reload when picked again.
    for (const s of this.series.values()) {
      if (s !== this.current) {
        s.load?.abort();
        s.older?.abort();
        this.series.delete(s.interval.secs);
      }
    }
    this.loadHistory(this.current);
    const signal = this.rest.signal;
    binanceGet<RawRestTicker>(`/api/v3/ticker/24hr?symbol=${this.query}`, signal)
      .then((t) => this.on.ticker(parseRestTicker(t)))
      .catch(() => {});
    binanceGet<RawBook>(`/api/v3/depth?symbol=${this.query}&limit=20`, signal)
      .then((b) => this.on.book(parseBook(b)))
      .catch(() => {});
    if (this.tradesOn) this.loadTrades();
  }

  private loadHistory(s: Series): void {
    s.load?.abort();
    window.clearTimeout(this.historyTimer);
    const load = new AbortController();
    s.load = load;
    s.ready = false;
    binanceGet<RawRestKline[]>(
      `/api/v3/klines?symbol=${this.query}&interval=${s.interval.api}&limit=${PAGE}`,
      AbortSignal.any([this.rest.signal, load.signal]),
    )
      .then((rows) => {
        if (load.signal.aborted) return;
        if (!s.replaceRecent(rows.map(parseKline))) s.exhausted = rows.length < PAGE;
        // Trades seen while loading; those already counted change nothing.
        for (const t of s.pending) s.add(t.price, t.time);
        s.pending = [];
        s.ready = true;
        if (s === this.current) this.publish();
      })
      .catch(() => {
        if (load.signal.aborted || this.disposed || this.suspended || s !== this.current) return;
        this.historyTimer = window.setTimeout(() => {
          if (!this.disposed && !this.suspended && s === this.current && !s.ready && !s.load) this.loadHistory(s);
        }, HISTORY_RETRY_MS);
      })
      .finally(() => {
        if (s.load === load) s.load = null;
      });
  }

  private loadTrades(): void {
    binanceGet<RawTrade[]>(`/api/v3/aggTrades?symbol=${this.query}&limit=40`, this.rest.signal)
      .then((list) => {
        this.pushTrades(list.map(parseTrade));
      })
      .catch(() => {});
  }
}

type RawRestKline = [number, string, string, string, string, ...unknown[]];

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

function parseKline([openMs, o, h, l, c]: RawRestKline): CandlePoint {
  return { time: openMs / 1000, open: +o, high: +h, low: +l, close: +c };
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
