// Live market data for one instrument: candle history per interval, plus a
// stream of every trade, the order book and the day's statistics, all from the
// app (whichever provider serves the instrument). Each trade moves the forming
// candle and the newest stretch of the line, so the chart moves with the
// market rather than once a second.

import type { CandlePoint, LivelinePoint } from "liveline";

import { api, type Book, type FeedState, type Interval, type LiveEvent, type Stats, type Trade } from "@/lib/api";

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

export interface MarketEvents {
  stats(stats: Stats): void;
  book(book: Book): void;
  /** Newest first. */
  trades(list: Trade[]): void;
  state(state: FeedState): void;
}

/** Candles per history request. */
const PAGE = 1000;
/** Bounds memory and per-frame work however far back someone scrolls. */
const MAX_CANDLES = 6000;
/** Line detail kept: ten minutes of one-second candles at four points a second. */
const MAX_TRAIL = 2400;
/** Trades held while an interval's history loads. */
const MAX_PENDING = 5000;
/** Trades arrive in batches every 100 ms; the chart eases between these updates. */
const CHART_FLUSH_MS = 100;
const TRADES_FLUSH_MS = 250;
const MAX_TRADES = 60;
const HISTORY_RETRY_MS = 5000;
/** Paging back retries after this, doubling per failure: a service left hammered may ban the IP. */
const OLDER_RETRY_MS = 2000;
const OLDER_RETRY_MAX_MS = 60_000;

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
  /** The history request in flight; a newer one or a reset replaces it. */
  load: object | null = null;
  older: object | null = null;
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
  private disposed = false;
  private suspended = false;
  /** Stops the current stream; null while none runs. */
  private stopStream: (() => void) | null = null;
  /** The stream went offline: catch up on history once it is live again. */
  private gap = false;
  private retryTimer: number | undefined;
  private historyTimer: number | undefined;
  private chartTimer: number | undefined;
  private tradesTimer: number | undefined;
  /** Every interval looked at so far: switching back is instant and animates. */
  private series = new Map<number, Series>();
  private current: Series;
  private price: number | null = null;
  private snapshot: ChartData | null = null;
  private listeners = new Set<() => void>();
  private tradesOn: boolean;
  private tradeList: Trade[] = [];

  constructor(
    private readonly id: string,
    interval: Interval,
    trades: boolean,
    private readonly on: MarketEvents,
  ) {
    this.current = this.seriesFor(interval);
    this.tradesOn = trades;
  }

  start(): void {
    this.loadSnapshot();
    this.connect();
  }

  dispose(): void {
    this.disposed = true;
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
    const older = {};
    s.older = older;
    api
      .chartHistory(this.id, s.interval.secs, first.time, limit)
      .then((page) => {
        if (s.older !== older || this.disposed) return;
        s.prepend(page);
        s.olderBackoff = 0;
        if (page.length < limit) s.exhausted = true;
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

  /** Nobody is looking: stop streaming until `resume`. */
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
    this.gap = false;
    this.on.state("connecting");
    let stopped = false;
    const stop = () => {
      stopped = true;
    };
    this.stopStream = stop;
    api
      .chartStream(this.id, (event) => {
        if (!stopped) this.handle(event);
      })
      .then((stopStream) => {
        if (stopped) stopStream();
        else if (this.stopStream === stop) {
          this.stopStream = () => {
            stop();
            stopStream();
          };
        }
      })
      .catch(() => {
        if (stopped || this.stopStream !== stop) return;
        this.stopStream = null;
        this.on.state("offline");
        this.retryTimer = window.setTimeout(() => this.connect(), HISTORY_RETRY_MS);
      });
  }

  /** Ends the current stream; its late events are ignored. */
  private drop(): void {
    this.stopStream?.();
    this.stopStream = null;
  }

  private clearTimers(): void {
    for (const timer of [this.retryTimer, this.historyTimer, this.chartTimer, this.tradesTimer]) {
      window.clearTimeout(timer);
    }
    this.retryTimer = this.historyTimer = this.chartTimer = this.tradesTimer = undefined;
  }

  private handle(event: LiveEvent): void {
    switch (event.kind) {
      case "state":
        this.on.state(event.state);
        if (event.state === "offline") {
          this.gap = true;
        } else if (event.state === "live") {
          if (this.gap) {
            this.gap = false;
            this.loadSnapshot();
          } else if (!this.current.ready && !this.current.load) {
            // A history request that failed earlier gets another go now.
            this.loadHistory(this.current);
          }
        }
        break;
      case "stats":
        this.on.stats(event.stats);
        break;
      case "book":
        this.on.book(event.book);
        break;
      case "trades":
        this.trades(event.trades);
        break;
    }
  }

  /** New trades, oldest first. */
  private trades(list: Trade[]): void {
    for (const trade of list) {
      const time = trade.time / 1000;
      this.price = trade.price;
      // Every cached interval stays live, not only the one on screen.
      for (const s of this.series.values()) {
        if (s.ready) {
          s.add(trade.price, time);
        } else if (s.load) {
          s.pending.push({ price: trade.price, time });
          if (s.pending.length > MAX_PENDING) s.pending = s.pending.slice(-MAX_PENDING / 2);
        }
      }
    }
    this.pushTrades(list);
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

  /** After a gap (start, reconnect, resume): fresh history and trades. The stream brings the rest. */
  private loadSnapshot(): void {
    // Until trades flow again the fresh history's close beats the last price seen.
    this.price = null;
    // Other intervals missed trades too; they reload when picked again.
    for (const s of this.series.values()) {
      if (s !== this.current) this.series.delete(s.interval.secs);
    }
    this.loadHistory(this.current);
    if (this.tradesOn) this.loadTrades();
  }

  private loadHistory(s: Series): void {
    window.clearTimeout(this.historyTimer);
    const load = {};
    s.load = load;
    s.older = null;
    s.ready = false;
    api
      .chartHistory(this.id, s.interval.secs, null, PAGE)
      .then((candles) => {
        if (s.load !== load || this.disposed) return;
        if (!s.replaceRecent(candles)) s.exhausted = candles.length < PAGE;
        // Trades seen while loading; those already counted change nothing.
        for (const t of s.pending) s.add(t.price, t.time);
        s.pending = [];
        s.ready = true;
        if (s === this.current) this.publish();
      })
      .catch(() => {
        if (s.load !== load || this.disposed || this.suspended || s !== this.current) return;
        this.historyTimer = window.setTimeout(() => {
          if (!this.disposed && !this.suspended && s === this.current && !s.ready && !s.load) this.loadHistory(s);
        }, HISTORY_RETRY_MS);
      })
      .finally(() => {
        if (s.load === load) s.load = null;
      });
  }

  private loadTrades(): void {
    api
      .chartTrades(this.id, 40)
      .then((list) => {
        if (!this.disposed) this.pushTrades(list);
      })
      .catch(() => {});
  }
}
