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
  /** Closes of the closed candles; the chart draws on to `price`. */
  line: LivelinePoint[];
  /** Latest trade price. */
  price: number | null;
  /** Nothing older can be paged in. */
  exhausted: boolean;
  /** History has loaded, even if it had no candles. */
  loaded: boolean;
}

export interface MarketEvents {
  stats(stats: Stats): void;
  /** One per ChartSpec.bookSteps entry, or just the one. */
  book(books: Book[]): void;
  /** Newest first. */
  trades(list: Trade[]): void;
  state(state: FeedState): void;
}

/** Candles per history request. */
const PAGE = 1000;
/** Bounds memory and per-frame work however far back someone scrolls. */
const MAX_CANDLES = 6000;
/** Trades held while an interval's history loads. */
const MAX_PENDING = 5000;
/** Trades arrive in batches every 100 ms; the chart eases between these updates. */
const CHART_FLUSH_MS = 100;
const TRADES_FLUSH_MS = 250;
const MAX_TRADES = 60;
const HISTORY_RETRY_MS = 5000;
/** How often the provider's latest candles are fetched for intervals it buckets itself. */
const REFRESH_MS = 30_000;
/** Paging back retries after this, doubling per failure: a service left hammered may ban the IP. */
const OLDER_RETRY_MS = 2000;
const OLDER_RETRY_MAX_MS = 60_000;

/** History and live state of one interval. */
class Series {
  candles: CandlePoint[] = [];
  live: CandlePoint | null = null;
  /** The line, rebuilt after the candles change. */
  private points: LivelinePoint[] | null = null;
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

  /**
   * One trade: fold it into the forming candle. Returns true when it belongs
   * to a candle only the provider can start.
   */
  add(price: number, time: number): boolean {
    const secs = this.interval.secs;
    const live = this.live;
    if (this.interval.aligned) {
      const bucket = Math.floor(time / secs) * secs;
      if (!live || bucket > live.time) {
        if (live) this.close(live);
        this.live = { time: bucket, open: price, high: price, low: price, close: price };
      } else if (bucket === live.time) {
        this.live = { ...live, high: Math.max(live.high, price), low: Math.min(live.low, price), close: price };
      } else {
        return false; // part of a candle the history already has
      }
    } else {
      // Trading sessions decide where these candles start: trades only move
      // the one forming, and one past its end waits for the provider's next.
      if (!live || time < live.time) return false;
      if (time >= live.time + secs) return true;
      this.live = { ...live, high: Math.max(live.high, price), low: Math.min(live.low, price), close: price };
    }
    return false;
  }

  /**
   * The provider's latest candles, oldest first, for an interval it buckets
   * itself: its versions replace ours, and a newer one starts the next candle.
   */
  mergeLatest(fresh: CandlePoint[]): void {
    for (const candle of fresh) {
      const live = this.live;
      if (live && candle.time < live.time) {
        const index = this.candles.findIndex((c) => c.time === candle.time);
        if (index >= 0) this.candles = this.candles.with(index, candle);
      } else if (live && candle.time === live.time) {
        this.live = candle;
      } else {
        if (live) this.close(live);
        this.live = candle;
      }
    }
    this.points = null;
  }

  private close(candle: CandlePoint): void {
    const candles = [...this.candles, candle];
    if (candles.length > MAX_CANDLES) {
      this.candles = candles.slice(-MAX_CANDLES);
      this.exhausted = false;
    } else {
      this.candles = candles;
    }
    this.points = null;
  }

  /**
   * The newest page from REST. Pages already scrolled in stay only if they
   * join up with it: after a long gap they would leave a hole that paging back
   * never reaches. Returns whether older pages were kept.
   */
  replaceRecent(fresh: CandlePoint[]): boolean {
    const live = fresh.pop() ?? null;
    const from = fresh[0]?.time ?? live?.time ?? Infinity;
    let kept = this.candles.filter((c) => c.time < from);
    const last = kept.at(-1);
    if (last && last.time + this.interval.secs < from) kept = [];
    this.candles = [...kept, ...fresh].slice(-MAX_CANDLES);
    this.live = live;
    this.points = null;
    return kept.length > 0;
  }

  prepend(page: CandlePoint[]): void {
    const first = this.candles[0]?.time ?? Infinity;
    this.candles = [...page.filter((c) => c.time < first), ...this.candles];
    this.points = null;
  }

  /**
   * The line: each closed candle's close at its center (where the
   * candle/line morph puts it), the same density before and after the window
   * opened; the chart carries it on to the latest price.
   */
  line(): LivelinePoint[] {
    const half = this.interval.secs / 2;
    this.points ??= this.candles.map((c) => ({ time: c.time + half, value: c.close }));
    return this.points;
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
  private refreshTimer: number | undefined;
  /** Series whose latest candles are being fetched. */
  private refreshing = new Set<Series>();
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
    this.keepRefreshing();
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
    this.keepRefreshing();
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
    window.clearInterval(this.refreshTimer);
    this.retryTimer = this.historyTimer = this.chartTimer = this.tradesTimer = this.refreshTimer = undefined;
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
        this.on.book(event.books);
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
        if (trade.extended && s.interval.regularOnly) continue;
        if (s.ready) {
          if (s.add(trade.price, time)) this.refresh(s);
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
      loaded: s.ready,
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
        const behind = s.pending.map((t) => s.add(t.price, t.time)).includes(true);
        s.pending = [];
        s.ready = true;
        if (behind) this.refresh(s);
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

  /** Every so often, the provider's latest candles for the intervals it buckets itself. */
  private keepRefreshing(): void {
    window.clearInterval(this.refreshTimer);
    this.refreshTimer = window.setInterval(() => {
      for (const s of this.series.values()) {
        if (!s.interval.aligned) this.refresh(s);
      }
    }, REFRESH_MS);
  }

  private refresh(s: Series): void {
    if (!s.ready || this.refreshing.has(s) || this.disposed || this.suspended) return;
    this.refreshing.add(s);
    api
      .chartHistory(this.id, s.interval.secs, null, 3)
      .then((candles) => {
        if (this.disposed || this.series.get(s.interval.secs) !== s || !s.ready) return;
        s.mergeLatest(candles);
        if (s === this.current) this.publish();
      })
      .catch(() => {})
      .finally(() => this.refreshing.delete(s));
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
