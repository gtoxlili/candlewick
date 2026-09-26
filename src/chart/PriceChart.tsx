import { useEffect, useRef, useState, useSyncExternalStore, type PointerEvent } from "react";
import { ChevronsRight } from "lucide-react";
import { Liveline, type CandlePoint } from "liveline";

import type { ChartMode, Interval } from "@/lib/api";
import { fmtPrice } from "@/lib/format";
import type { ChartData, Market } from "./market";
import type { ChartColors } from "./palette";
import { load, store } from "./prefs";
import { clampBars, DEFAULT_BARS, Viewport } from "./viewport";

export interface VisibleStats {
  /** "近 1 小时" while following the live edge, otherwise the stretch shown. */
  label: string;
  changePct: number;
  amplitudePct: number;
  live: boolean;
}

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86_400;
/** Pointer travel before a press becomes a drag, so clicks stay clicks. */
const DRAG_SLOP = 4;
/**
 * A double-click this soon after a drag is two quick drags that macOS counted
 * as clicks, not a request to reset.
 */
const DRAG_DBLCLICK_MS = 500;

let gestured = load("chart.gestured", (raw) => raw === "1", false);

/** Someone has found the gestures: the hint can go, for good. */
function rememberGesture(): void {
  if (gestured) return;
  gestured = true;
  store("chart.gestured", "1");
}

interface Press {
  id: number;
  x: number;
  dragging: boolean;
  samples: { x: number; t: number }[];
}

/**
 * The live chart: Liveline in candle mode (morphing to a line), panned by
 * dragging or swiping, zoomed by scrolling or pinching. Double-click returns
 * to the live edge at the default zoom.
 */
export function PriceChart(props: {
  market: Market;
  interval: Interval;
  mode: ChartMode;
  colors: ChartColors;
  /** Changes when the trend colors do; Liveline resolves them on mount. */
  paletteKey: number;
  decimals: number;
  offline: boolean;
  /** The provider's name, for the offline message. */
  source: string;
  /** Seconds east of UTC at which daily candles open. */
  dayOffset: number;
  onStats: (stats: VisibleStats | null) => void;
}) {
  const { market, interval, onStats } = props;
  const data = useSyncExternalStore(market.subscribeChart, market.chartData);
  const [viewport] = useState(
    () => new Viewport(interval.secs, load("chart.bars", (raw) => clampBars(Number(raw)) || undefined, DEFAULT_BARS)),
  );
  const [live, setLive] = useState(true);
  const [dragging, setDragging] = useState(false);
  const [hint, setHint] = useState(!gestured);
  const surface = useRef<HTMLDivElement>(null);
  const press = useRef<Press | null>(null);
  const lastDragEnd = useRef(Number.NEGATIVE_INFINITY);

  useEffect(() => {
    let saveTimer: number | undefined;
    viewport.listen({
      live: setLive,
      older: () => market.loadOlder(),
      zoom: (bars) => {
        window.clearTimeout(saveTimer);
        saveTimer = window.setTimeout(() => store("chart.bars", String(Math.round(bars))), 400);
      },
    });
    return () => {
      window.clearTimeout(saveTimer);
      viewport.listen(null);
    };
  }, [viewport, market]);

  useEffect(() => () => viewport.dispose(), [viewport]);

  // Another pair, or another candle size: back to the live edge.
  useEffect(() => {
    market.selectInterval(interval);
    viewport.selectInterval(interval.secs);
  }, [market, viewport, interval]);

  useEffect(() => {
    if (!data || data.interval !== interval.secs) return;
    viewport.setHistory(data.candles[0]?.time ?? data.live?.time ?? null, data.exhausted);
  }, [viewport, data, interval]);

  // Scroll, swipe and pinch. Registered by hand: React's wheel listener is
  // passive, and the page must not rubber-band under the chart.
  useEffect(() => {
    const el = surface.current;
    if (!el) return;
    let pinching = false;
    let lastScale = 1;
    const localX = (clientX: number) => clientX - el.getBoundingClientRect().left - el.clientLeft;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      if (pinching) return;
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? el.clientWidth : 1;
      const dx = e.deltaX * unit;
      const dy = e.deltaY * unit;
      if (e.ctrlKey) {
        viewport.zoom(Math.exp(dy * 0.01), localX(e.clientX)); // pinch, as Chromium reports it
      } else if (Math.abs(dx) > Math.abs(dy)) {
        viewport.scrollBy(dx);
      } else {
        viewport.zoom(Math.exp(dy * 0.002), localX(e.clientX));
      }
      rememberGesture();
      setHint(false);
    };
    // WebKit reports a trackpad pinch as gesture events with a running scale.
    const onGestureStart = (e: Event) => {
      e.preventDefault();
      pinching = true;
      lastScale = 1;
    };
    const onGestureChange = (e: Event) => {
      e.preventDefault();
      const { scale, clientX } = e as Event & { scale: number; clientX: number };
      viewport.zoom(lastScale / scale, localX(clientX));
      lastScale = scale;
      rememberGesture();
      setHint(false);
    };
    const onGestureEnd = (e: Event) => {
      e.preventDefault();
      pinching = false;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    el.addEventListener("gesturestart", onGestureStart);
    el.addEventListener("gesturechange", onGestureChange);
    el.addEventListener("gestureend", onGestureEnd);
    return () => {
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("gesturestart", onGestureStart);
      el.removeEventListener("gesturechange", onGestureChange);
      el.removeEventListener("gestureend", onGestureEnd);
    };
  }, [viewport]);

  // What the visible stretch did, for the line under the price.
  useEffect(() => {
    let last = "";
    const report = () => {
      const stats = visibleStats(market.chartData(), viewport);
      const key = JSON.stringify(stats);
      if (key === last) return;
      last = key;
      onStats(stats);
    };
    report();
    const timer = window.setInterval(report, 500);
    return () => window.clearInterval(timer);
  }, [market, viewport, onStats]);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    // A drag released outside the chart without capture never saw its pointerup.
    setDragging(false);
    press.current = { id: e.pointerId, x: e.clientX, dragging: false, samples: [] };
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    if (!p.dragging) {
      if (Math.abs(e.clientX - p.x) < DRAG_SLOP) return;
      p.dragging = true;
      p.x = e.clientX;
      try {
        e.currentTarget.setPointerCapture(e.pointerId);
      } catch {
        // Already released; the drag goes on while the pointer stays over the chart.
      }
      viewport.dragStart();
      setDragging(true);
      rememberGesture();
      setHint(false);
    }
    viewport.dragTo(e.clientX - p.x);
    p.samples.push({ x: e.clientX, t: e.timeStamp });
    if (p.samples.length > 8) p.samples.shift();
  };

  const onPointerEnd = (e: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    press.current = null;
    if (!p.dragging) return;
    setDragging(false);
    lastDragEnd.current = e.timeStamp;
    // Speed over the last ~100 ms decides whether the view glides on.
    const from = p.samples.find((s) => e.timeStamp - s.t <= 100);
    const elapsed = from ? e.timeStamp - from.t : 0;
    viewport.dragEnd(from && elapsed > 0 && e.type === "pointerup" ? (e.clientX - from.x) / elapsed : 0);
  };

  const current = data?.interval === interval.secs ? data : null;
  const ready = !!current && (current.candles.length > 0 || current.live !== null);
  const line = current?.line ?? [];
  const price = current?.price ?? current?.live?.close ?? 0;

  return (
    <div
      ref={surface}
      className="relative mx-4 min-h-0 flex-1 touch-none overflow-hidden rounded-2xl border bg-white/45 select-none dark:bg-white/3"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onDoubleClick={(e) => {
        if (e.timeStamp - lastDragEnd.current > DRAG_DBLCLICK_MS) viewport.reset();
      }}
    >
      <Liveline
        key={props.paletteKey}
        mode="candle"
        view={viewport.view}
        data={line}
        value={price}
        candles={current?.candles ?? []}
        candleWidth={interval.secs}
        liveCandle={current?.live ?? undefined}
        lineMode={props.mode === "line"}
        lineData={line}
        lineValue={price}
        theme={props.colors.dark ? "dark" : "light"}
        color={props.colors.accent}
        loading={!ready && !current?.loaded}
        scrub={!dragging}
        cursor={dragging ? "grabbing" : "crosshair"}
        formatValue={(v) => fmtPrice(v, props.decimals)}
        formatTime={(t) => axisLabel(t, viewport.view.span)}
        formatHoverTime={(t) => candleLabel(t, interval.secs, props.dayOffset)}
        padding={{ top: 16, bottom: 28, left: 14 }}
      />

      {hint && ready && (
        <p className="pointer-events-none absolute top-2.5 left-4 text-[11px] text-muted-foreground animate-in fade-in delay-1000 duration-700 fill-mode-both">
          拖动查看更早的走势 · 滚动或双指缩放 · 双击复位
        </p>
      )}

      {!live && (
        <button
          type="button"
          onClick={() => viewport.goLive()}
          onPointerDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
          className="absolute right-23 bottom-9 flex h-7 items-center gap-1 rounded-full border bg-popover pr-2 pl-3 text-xs text-foreground shadow-[0_2px_8px_rgb(0_0_0/0.12)] backdrop-blur-xl animate-in fade-in slide-in-from-right-2 duration-200 hover:bg-accent"
        >
          回到最新
          <ChevronsRight className="size-3.5 text-muted-foreground" />
        </button>
      )}

      {!ready && current?.loaded && (
        <p className="absolute inset-0 flex items-center justify-center text-xs text-muted-foreground">暂无 K 线数据</p>
      )}

      {!ready && props.offline && (
        <p className="absolute inset-x-0 bottom-3 text-center text-xs text-muted-foreground">
          无法连接{props.source}，正在重试…
        </p>
      )}
    </div>
  );
}

function visibleStats(data: ChartData | null, viewport: Viewport): VisibleStats | null {
  const { left, right, span } = viewport.view;
  if (!data || left === undefined || right === undefined) return null;
  const candles: CandlePoint[] = data.live ? [...data.candles, data.live] : data.candles;
  let first = -1;
  let last = -1;
  let high = -Infinity;
  let low = Infinity;
  for (let i = 0; i < candles.length; i++) {
    const c = candles[i];
    if (c.time + data.interval <= left || c.time >= right) continue;
    if (first < 0) first = i;
    last = i;
    high = Math.max(high, c.high);
    low = Math.min(low, c.low);
  }
  if (first < 0 || last <= first) return null;
  const open = candles[first].open;
  const close = candles[last].close;
  return {
    label: viewport.live ? `近 ${fmtSpan(span)}` : fmtRange(left, right),
    changePct: round2(((close - open) / open) * 100),
    amplitudePct: round2(((high - low) / low) * 100),
    live: viewport.live,
  };
}

const round2 = (n: number) => Math.round(n * 100) / 100;

/** A duration in words: "45 秒", "15 分钟", "1.5 小时", "60 天". */
function fmtSpan(secs: number): string {
  const tenths = (n: number) => (n < 10 ? Math.round(n * 10) / 10 : Math.round(n));
  if (secs < MINUTE) return `${Math.round(secs)} 秒`;
  if (secs < HOUR) return `${Math.round(secs / MINUTE)} 分钟`;
  if (secs < 2 * DAY) return `${tenths(secs / HOUR)} 小时`;
  return `${tenths(secs / DAY)} 天`;
}

const pad2 = (n: number) => String(n).padStart(2, "0");
const hm = (d: Date) => `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
const hms = (d: Date) => `${hm(d)}:${pad2(d.getSeconds())}`;

function isToday(d: Date): boolean {
  return d.toDateString() === new Date().toDateString();
}

/** The stretch shown while browsing history: times within a day or two, dates beyond. */
function fmtRange(left: number, right: number): string {
  const a = new Date(left * 1000);
  const b = new Date(right * 1000);
  const span = right - left;
  if (span < 2 * DAY) {
    const day = (d: Date) => (isToday(d) ? "" : `${d.getMonth() + 1}月${d.getDate()}日 `);
    const time = span < 10 * MINUTE ? hms : hm;
    return a.toDateString() === b.toDateString()
      ? `${day(a)}${time(a)}–${time(b)}`
      : `${day(a)}${time(a)} – ${day(b)}${time(b)}`;
  }
  const thisYear = new Date().getFullYear();
  const date = (d: Date) =>
    `${d.getFullYear() === thisYear ? "" : `${d.getFullYear()}年`}${d.getMonth() + 1}月${d.getDate()}日`;
  return `${date(a)} – ${date(b)}`;
}

/** Time axis labels; called for every label on every frame, so no Intl. */
function axisLabel(t: number, span: number): string {
  const d = new Date(t * 1000);
  // Midnight, and every label of a multi-day view, reads as a date.
  if (span > 2 * DAY || (d.getHours() === 0 && d.getMinutes() === 0 && d.getSeconds() === 0)) {
    return `${d.getMonth() + 1}/${d.getDate()}`;
  }
  // Labels land on whole minutes beyond about five minutes of view.
  return span <= 5 * MINUTE ? hms(d) : hm(d);
}

/** The hovered candle's start. Daily candles carry the date of the day they open, where they open. */
function candleLabel(t: number, secs: number, dayOffset: number): string {
  if (secs >= DAY) {
    const day = new Date((t + dayOffset) * 1000);
    return `${day.getUTCFullYear()}/${day.getUTCMonth() + 1}/${day.getUTCDate()}`;
  }
  const d = new Date(t * 1000);
  const time = secs < MINUTE ? hms(d) : hm(d);
  return isToday(d) ? time : `${d.getMonth() + 1}/${d.getDate()} ${time}`;
}
