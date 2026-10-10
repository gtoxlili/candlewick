import { useEffect, useRef, useState, useSyncExternalStore, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { ChevronsRight } from "lucide-react";
import { Liveline, type CandlePoint, type LivelineFrame } from "liveline";

import type { ChartMode, Interval, ProviderId } from "@/lib/api";
import { fmtDuration, fmtPrice } from "@/lib/format";
import { load, store } from "@/lib/prefs";
import type { Analysis } from "./analysis/analysis";
import { indicatorLabel } from "./analysis/indicators";
import { Overlay, indicatorColor, type Labels } from "./analysis/overlay";
import { HoverCard, IndicatorLegend, type HoverCardHandle, type LegendHandle, type Reading } from "./HoverCard";
import { axisLabel, candleLabel } from "./labels";
import type { ChartData, Market } from "./market";
import type { ChartColors } from "./palette";
import { clampBars, DEFAULT_BARS, Viewport } from "./viewport";

/** What the stretch on screen did: the last `span` seconds while following
 * the live edge, otherwise `from` to `to` (epoch seconds). */
export interface VisibleStats {
  live: boolean;
  span: number;
  from: number;
  to: number;
  changePct: number;
  amplitudePct: number;
}

/** Pointer travel before a press becomes a drag, so clicks stay clicks. */
const DRAG_SLOP = 4;
/**
 * A double-click this soon after a drag is two quick drags that the system
 * counted as clicks, not a request to reset.
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

/** Which tool a letter with Alt picks. */
const TOOL_KEYS = { KeyV: "cursor", KeyH: "level", KeyT: "trend", KeyF: "fib", KeyM: "measure" } as const;

/**
 * The live chart: Liveline in candle mode (morphing to a line), panned by
 * dragging or swiping, zoomed by scrolling or pinching, double-click back
 * to the live edge at the default zoom. Over it, the analysis layer: the
 * crosshair and hover card, the indicators, and the drawings and their
 * gestures, all drawn once per frame Liveline reports.
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
  /** For the offline message. */
  source: ProviderId;
  /** Seconds east of UTC at which daily candles open. */
  dayOffset: number;
  analysis: Analysis;
  onStats: (stats: VisibleStats | null) => void;
}) {
  const { t, i18n } = useTranslation();
  const { market, interval, analysis, onStats } = props;
  const data = useSyncExternalStore(market.subscribeChart, market.chartData);
  const study = useSyncExternalStore(analysis.subscribe, analysis.snapshot);
  const [viewport] = useState(
    () => new Viewport(interval.secs, load("chart.bars", (raw) => clampBars(Number(raw)) || undefined, DEFAULT_BARS)),
  );
  const [live, setLive] = useState(true);
  const [panning, setPanning] = useState(false);
  const [hint, setHint] = useState(!gestured);
  const surface = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const overlay = useRef<Overlay | null>(null);
  const card = useRef<HoverCardHandle>(null);
  const legend = useRef<LegendHandle>(null);
  const frame = useRef<LivelineFrame | null>(null);
  const size = useRef({ w: 0, h: 0 });
  const press = useRef<Press | null>(null);
  const lastDragEnd = useRef(Number.NEGATIVE_INFINITY);

  // What the frame handler reads: the latest render's values, without
  // re-registering it on every change.
  const current = useRef({ decimals: props.decimals, colors: props.colors, interval, panning, dayOffset: props.dayOffset, locale: i18n.language, t });
  current.current = { decimals: props.decimals, colors: props.colors, interval, panning, dayOffset: props.dayOffset, locale: i18n.language, t };

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

  // The overlay canvas tracks the chart's size at the device's resolution.
  useEffect(() => {
    const el = surface.current;
    const target = canvas.current;
    if (!el || !target) return;
    const painter = new Overlay(target);
    overlay.current = painter;
    const measure = () => {
      size.current = { w: el.clientWidth, h: el.clientHeight };
      painter.resize(el.clientWidth, el.clientHeight);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => {
      observer.disconnect();
      overlay.current = null;
    };
  }, []);

  // The indicators follow the candles on screen, the forming one included.
  const indicatorInput = data && data.interval === interval.secs ? data : null;
  useEffect(() => {
    const candles: CandlePoint[] = indicatorInput
      ? indicatorInput.live
        ? [...indicatorInput.candles, indicatorInput.live]
        : indicatorInput.candles
      : [];
    analysis.setCandles(candles);
  }, [analysis, indicatorInput]);

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

  // Keys: Escape backs out, Delete removes the selected drawing, ⌘Z brings
  // it back, Alt and a letter picks a tool.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof Element && e.target.closest("input, textarea, select, [contenteditable]")) return;
      if (e.key === "Escape") {
        if (analysis.cancel()) e.preventDefault();
      } else if (e.key === "Backspace" || e.key === "Delete") {
        if (analysis.selected !== null) {
          e.preventDefault();
          analysis.removeSelected();
        }
      } else if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.code === "KeyZ") {
        e.preventDefault();
        analysis.undo();
      } else if (e.altKey && !e.metaKey && !e.ctrlKey && e.code in TOOL_KEYS) {
        e.preventDefault();
        analysis.setTool(TOOL_KEYS[e.code as keyof typeof TOOL_KEYS]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [analysis]);

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

  /** Every frame Liveline draws: the overlay, the hover card and the legend follow it. */
  const onFrame = (next: LivelineFrame) => {
    frame.current = next;
    const now = current.current;
    const labels: Labels = {
      time: (time) => candleLabel(time, now.interval.secs, now.dayOffset),
      stretch: (bars, secs) => `${now.t("chart.bars", { count: bars })} · ${fmtDuration(secs, now.locale)}`,
    };
    const busy = now.panning || analysis.snapshot().dragging;
    overlay.current?.draw(next, {
      drawings: analysis.drawings,
      selected: analysis.selected,
      hovered: analysis.hovered,
      draft: analysis.draft,
      pointer: analysis.pointer,
      snapped: analysis.snapped,
      crosshair: !busy,
      indicators: analysis.series,
      candles: analysis.candles,
      interval: now.interval.secs,
      decimals: now.decimals,
      colors: now.colors,
      labels,
    });
    const index = next.hover ? analysis.indexOf(next.hover.candle.time) : null;
    const readings: Reading[] = analysis.valuesAt(index).map(({ spec, value }) => ({
      id: spec.id,
      label: indicatorLabel(spec),
      color: indicatorColor(spec, now.colors),
      value,
    }));
    legend.current?.update(readings);
    // Only while the pointer is in the plot: Liveline keeps a hovered candle
    // from the x alone, over the axes too.
    const showCard = next.hover && analysis.snapped && analysis.tool === "cursor" && !busy && !analysis.hovered;
    card.current?.update(
      showCard && next.hover && analysis.pointer
        ? {
            x: analysis.pointer.x,
            y: analysis.pointer.y,
            width: size.current.w,
            height: size.current.h,
            candle: next.hover.candle,
            time: labels.time(next.hover.candle.time),
            readings,
          }
        : null,
    );
  };

  const local = (e: PointerEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const rect = el.getBoundingClientRect();
    return { x: e.clientX - rect.left - el.clientLeft, y: e.clientY - rect.top - el.clientTop };
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    // A drag released outside the chart without capture never saw its pointerup.
    setPanning(false);
    const became = analysis.pointerDown(local(e), frame.current, interval.secs, e.altKey);
    if (became !== "pan") {
      press.current = null;
      if (became === "drag") {
        try {
          e.currentTarget.setPointerCapture(e.pointerId);
        } catch {
          // Already released; the drag goes on while the pointer stays over the chart.
        }
      }
      return;
    }
    press.current = { id: e.pointerId, x: e.clientX, dragging: false, samples: [] };
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    analysis.pointerMove(local(e), frame.current, interval.secs, e.altKey);
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
      setPanning(true);
      rememberGesture();
      setHint(false);
    }
    viewport.dragTo(e.clientX - p.x);
    p.samples.push({ x: e.clientX, t: e.timeStamp });
    if (p.samples.length > 8) p.samples.shift();
  };

  const onPointerEnd = (e: PointerEvent<HTMLDivElement>) => {
    analysis.pointerUp();
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    press.current = null;
    if (!p.dragging) return;
    setPanning(false);
    lastDragEnd.current = e.timeStamp;
    // Speed over the last ~100 ms decides whether the view glides on.
    const from = p.samples.find((s) => e.timeStamp - s.t <= 100);
    const elapsed = from ? e.timeStamp - from.t : 0;
    viewport.dragEnd(from && elapsed > 0 && e.type === "pointerup" ? (e.clientX - from.x) / elapsed : 0);
  };

  const chart = data?.interval === interval.secs ? data : null;
  const ready = !!chart && (chart.candles.length > 0 || chart.live !== null);
  const line = chart?.line ?? [];
  const price = chart?.price ?? chart?.live?.close ?? 0;
  // Room on the right for the price labels, which a token priced in ten
  // decimals makes twice as wide as a dollar stock's.
  const axisWidth = Math.max(64, 16 + 6.8 * fmtPrice(price || 1, props.decimals).length);
  const readings: Reading[] = study.indicators
    .filter((spec) => spec.on)
    .map((spec) => ({ id: spec.id, label: indicatorLabel(spec), color: indicatorColor(spec, props.colors), value: null }));
  const cursor =
    study.tool !== "cursor"
      ? "crosshair"
      : panning || study.dragging
        ? "grabbing"
        : study.hovered === "body"
          ? "move"
          : study.hovered !== null
            ? "grab"
            : "crosshair";

  return (
    <div
      ref={surface}
      className="panel relative mx-4 min-h-0 flex-1 touch-none overflow-hidden select-none"
      style={{ cursor }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onPointerLeave={() => analysis.pointerLeave()}
      onDoubleClick={(e) => {
        if (study.tool !== "cursor" || study.hovered !== null) return;
        if (e.timeStamp - lastDragEnd.current > DRAG_DBLCLICK_MS) viewport.reset();
      }}
    >
      <Liveline
        key={props.paletteKey}
        mode="candle"
        view={viewport.view}
        data={line}
        value={price}
        candles={chart?.candles ?? []}
        candleWidth={interval.secs}
        liveCandle={chart?.live ?? undefined}
        lineMode={props.mode === "line"}
        lineData={line}
        lineValue={price}
        theme={props.colors.dark ? "dark" : "light"}
        color={props.colors.accent}
        loading={!ready && !chart?.loaded}
        scrub={!panning && !study.dragging}
        crosshair={false}
        onFrame={onFrame}
        cursor={cursor}
        formatValue={(v) => fmtPrice(v, props.decimals)}
        formatTime={(t) => axisLabel(t, viewport.view.span)}
        padding={{ top: 16, bottom: 28, left: 14, right: axisWidth }}
      />
      <canvas ref={canvas} aria-hidden className="pointer-events-none absolute inset-0 size-full" />
      <HoverCard ref={card} decimals={props.decimals} readings={readings} />
      <IndicatorLegend ref={legend} readings={readings} decimals={props.decimals} />

      {hint && ready && !study.drafting && (
        <p className="pointer-events-none absolute bottom-9 left-4 text-2xs text-muted-foreground animate-in fade-in delay-1000 duration-700 fill-mode-both">
          {t("chart.hint")}
        </p>
      )}

      {study.drafting && (
        <p className="pointer-events-none absolute bottom-9 left-4 text-2xs text-muted-foreground animate-in fade-in duration-200">
          {t("chart.drawingHint")}
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
          {t("chart.backToLive")}
          <ChevronsRight className="size-3.5 text-muted-foreground" />
        </button>
      )}

      {!ready && chart?.loaded && (
        <p className="absolute inset-0 flex items-center justify-center text-xs text-muted-foreground">
          {t("chart.noCandles")}
        </p>
      )}

      {!ready && props.offline && (
        <p className="absolute inset-x-0 bottom-3 text-center text-xs text-muted-foreground">
          {t("chart.unreachable", { source: t(`common.provider.${props.source}`) })}
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
    live: viewport.live,
    // Whole seconds, and the ends only off the live edge (where they move
    // with the clock): a stretch that stays put reports nothing new.
    span: Math.round(span),
    from: viewport.live ? 0 : Math.round(left),
    to: viewport.live ? 0 : Math.round(right),
    changePct: round2(((close - open) / open) * 100),
    amplitudePct: round2(((high - low) / low) * 100),
  };
}

const round2 = (n: number) => Math.round(n * 100) / 100;
