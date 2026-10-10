// Draws over Liveline's canvas, once per frame it reports: the indicator
// lines, the drawings, the one being drawn, and the crosshair with its
// price and time tags. Everything is placed through the frame's eased
// ranges, so it moves with the candles rather than ahead of them.

import type { CandlePoint, LivelineFrame } from "liveline";

import { fmtPct, fmtPrice, fmtSigned } from "@/lib/format";
import { rgba, type ChartColors, type Rgb } from "../palette";
import { fibSpan, inPlot, mapping, snappedToCandle, type Hit, type Mapping, type Plot, type Point } from "./geometry";
import type { IndicatorSeries, IndicatorSpec } from "./indicators";
import { FIB_LEVELS, type Anchor, type Drawing, type DrawingKind } from "./model";

export interface Labels {
  /** A candle's start, for the time tag. */
  time(t: number): string;
  /** "24 bars · 2 hours", for a measured stretch. */
  stretch(bars: number, secs: number): string;
}

export interface OverlayState {
  drawings: readonly Drawing[];
  selected: string | null;
  hovered: Hit | null;
  /** The drawing in progress: the points placed; its next end follows `snapped`. */
  draft: { kind: DrawingKind; points: Anchor[] } | null;
  pointer: Point | null;
  /** Where the pointer lands in the chart, snapped to the candle under it. */
  snapped: Anchor | null;
  /** Show the crosshair (not while panning or dragging). */
  crosshair: boolean;
  indicators: readonly IndicatorSeries[];
  /** The candles the indicators were computed over, oldest first. */
  candles: readonly CandlePoint[];
  interval: number;
  decimals: number;
  colors: ChartColors;
  labels: Labels;
}

const FONT = '500 11px -apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';
const SMALL_FONT = FONT.replace("11px", "10px");
const TAG_H = 18;
const HANDLE_R = 4.5;

/** The hue of an indicator, stable per slot so the legend never recolors. */
export function indicatorColor(spec: IndicatorSpec, colors: ChartColors): Rgb {
  switch (spec.id) {
    case "ma1":
      return colors.violet;
    case "ma2":
      return colors.amber;
    case "ma3":
      return colors.mint;
    case "boll":
      return colors.glow;
    default:
      return colors.ink;
  }
}

/** First candle whose stretch reaches `time`, by binary search. */
function firstFrom(candles: readonly CandlePoint[], time: number, interval: number): number {
  let lo = 0;
  let hi = candles.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (candles[mid].time + interval < time) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

export class Overlay {
  private readonly ctx: CanvasRenderingContext2D;
  private dpr = 1;
  private w = 0;
  private h = 0;

  constructor(private readonly canvas: HTMLCanvasElement) {
    this.ctx = canvas.getContext("2d")!;
  }

  resize(w: number, h: number): void {
    this.dpr = window.devicePixelRatio || 1;
    this.w = w;
    this.h = h;
    this.canvas.width = Math.round(w * this.dpr);
    this.canvas.height = Math.round(h * this.dpr);
  }

  draw(frame: LivelineFrame, state: OverlayState): void {
    const { ctx } = this;
    if (this.w === 0 || this.h === 0) return;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, this.w, this.h);
    const map = mapping(frame);
    const { plot } = map;
    if (plot.w <= 0 || plot.h <= 0) return;

    ctx.save();
    ctx.beginPath();
    ctx.rect(plot.x, plot.y, plot.w, plot.h);
    ctx.clip();
    this.indicators(frame, map, state);
    for (const drawing of state.drawings) {
      const selected = drawing.id === state.selected;
      const hovered = state.hovered?.id === drawing.id;
      this.drawing(drawing, map, state, selected ? state.colors.glow : state.colors.violet, hovered || selected, false);
    }
    if (state.draft && state.snapped) {
      const points = [...state.draft.points, state.snapped];
      const [a, b] = points;
      const draft: Drawing =
        state.draft.kind === "level"
          ? { id: "draft", kind: "level", price: a.price }
          : { id: "draft", kind: state.draft.kind, a, b: b ?? a };
      this.drawing(draft, map, state, state.colors.glow, true, true);
    }
    ctx.restore();

    for (const drawing of state.drawings) {
      if (drawing.kind === "level") {
        const selected = drawing.id === state.selected;
        this.priceTag(map.y(drawing.price), fmtPrice(drawing.price, state.decimals), selected ? state.colors.glow : state.colors.violet, plot);
      }
    }
    if (state.selected) {
      const drawing = state.drawings.find((d) => d.id === state.selected);
      if (drawing) this.handles(drawing, map, state.colors);
    }
    this.crosshair(frame, map, state);
  }

  private indicators(frame: LivelineFrame, map: Mapping, state: OverlayState): void {
    const { ctx } = this;
    const { candles, interval } = state;
    if (candles.length === 0) return;
    const half = interval / 2;
    const from = Math.max(0, firstFrom(candles, frame.left, interval) - 1);
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    for (const series of state.indicators) {
      const color = indicatorColor(series.spec, state.colors);
      const n = Math.min(candles.length, series.values.length);
      if (series.upper && series.lower) {
        // The band: a soft fill between the outer lines, which are drawn thin.
        ctx.beginPath();
        let started = false;
        for (let i = from; i < n; i++) {
          const v = series.upper[i];
          if (!Number.isFinite(v)) continue;
          const x = map.x(candles[i].time + half);
          if (x > map.plot.x + map.plot.w + 20) break;
          started ? ctx.lineTo(x, map.y(v)) : ctx.moveTo(x, map.y(v));
          started = true;
        }
        for (let i = n - 1; i >= from; i--) {
          const v = series.lower[i];
          if (!Number.isFinite(v)) continue;
          ctx.lineTo(map.x(candles[i].time + half), map.y(v));
        }
        ctx.closePath();
        ctx.fillStyle = rgba(color, 0.07);
        ctx.fill();
        for (const band of [series.upper, series.lower]) {
          this.path(band, candles, from, n, half, map);
          ctx.strokeStyle = rgba(color, 0.55);
          ctx.lineWidth = 1;
          ctx.stroke();
        }
        this.path(series.values, candles, from, n, half, map);
        ctx.setLineDash([2, 4]);
        ctx.strokeStyle = rgba(color, 0.7);
        ctx.lineWidth = 1;
        ctx.stroke();
        ctx.setLineDash([]);
        continue;
      }
      this.path(series.values, candles, from, n, half, map);
      ctx.strokeStyle = rgba(color, 0.9);
      ctx.lineWidth = 1.25;
      ctx.stroke();
    }
  }

  private path(values: Float64Array, candles: readonly CandlePoint[], from: number, to: number, half: number, map: Mapping): void {
    const { ctx } = this;
    ctx.beginPath();
    let started = false;
    const limit = map.plot.x + map.plot.w + 20;
    for (let i = from; i < to; i++) {
      const v = values[i];
      if (!Number.isFinite(v)) continue;
      const x = map.x(candles[i].time + half);
      if (x > limit) break;
      started ? ctx.lineTo(x, map.y(v)) : ctx.moveTo(x, map.y(v));
      started = true;
    }
  }

  private drawing(d: Drawing, map: Mapping, state: OverlayState, color: Rgb, lit: boolean, draft: boolean): void {
    const { ctx } = this;
    const { plot } = map;
    const alpha = lit ? 1 : 0.8;
    const width = lit ? 1.5 : 1;
    ctx.setLineDash(draft ? [4, 4] : []);
    switch (d.kind) {
      case "level": {
        const y = map.y(d.price);
        this.line(plot.x, y, plot.x + plot.w, y, rgba(color, alpha), width);
        break;
      }
      case "trend": {
        this.line(map.x(d.a.time), map.y(d.a.price), map.x(d.b.time), map.y(d.b.price), rgba(color, alpha), width);
        break;
      }
      case "fib":
        this.fib(d.a, d.b, map, state, color, lit);
        break;
      case "measure":
        this.measure(d.a, d.b, map, state, lit);
        break;
    }
    ctx.setLineDash([]);
  }

  private line(x0: number, y0: number, x1: number, y1: number, style: string, width: number): void {
    const { ctx } = this;
    ctx.beginPath();
    ctx.moveTo(x0, y0);
    ctx.lineTo(x1, y1);
    ctx.strokeStyle = style;
    ctx.lineWidth = width;
    ctx.stroke();
  }

  private fib(a: Anchor, b: Anchor, map: Mapping, state: OverlayState, color: Rgb, lit: boolean): void {
    const { ctx } = this;
    const pa = { x: map.x(a.time), y: map.y(a.price) };
    const pb = { x: map.x(b.time), y: map.y(b.price) };
    const [x0, x1] = fibSpan(pa, pb, map.plot);
    const ys = FIB_LEVELS.map((level) => pa.y + (pb.y - pa.y) * level);
    // Bands between the levels, deepening toward the far end of the swing.
    for (let i = 1; i < ys.length; i++) {
      ctx.fillStyle = rgba(color, 0.035 + 0.012 * i);
      ctx.fillRect(x0, Math.min(ys[i - 1], ys[i]), x1 - x0, Math.abs(ys[i] - ys[i - 1]));
    }
    ctx.setLineDash([3, 5]);
    this.line(pa.x, pa.y, pb.x, pb.y, rgba(color, 0.35), 1);
    ctx.setLineDash([]);
    ctx.font = SMALL_FONT;
    ctx.textBaseline = "bottom";
    ctx.textAlign = "left";
    FIB_LEVELS.forEach((level, i) => {
      const y = ys[i];
      const edge = level === 0 || level === 1;
      this.line(x0, y, x1, y, rgba(color, edge ? 0.85 : 0.55), edge || lit ? 1 : 0.75);
      const price = a.price + (b.price - a.price) * level;
      const text = `${level.toFixed(3).replace(/\.?0+$/, "")}  ${fmtPrice(price, state.decimals)}`;
      ctx.fillStyle = rgba(color, 0.95);
      ctx.fillText(text, x0 + 6, y - 3);
    });
  }

  private measure(a: Anchor, b: Anchor, map: Mapping, state: OverlayState, lit: boolean): void {
    const { ctx } = this;
    const up = b.price >= a.price;
    const color = up ? state.colors.up : state.colors.down;
    const x = Math.min(map.x(a.time), map.x(b.time));
    const y = Math.min(map.y(a.price), map.y(b.price));
    const w = Math.abs(map.x(b.time) - map.x(a.time));
    const h = Math.abs(map.y(b.price) - map.y(a.price));
    ctx.fillStyle = rgba(color, lit ? 0.12 : 0.09);
    ctx.fillRect(x, y, w, h);
    ctx.strokeStyle = rgba(color, 0.6);
    ctx.lineWidth = 1;
    ctx.strokeRect(x + 0.5, y + 0.5, Math.max(0, w - 1), Math.max(0, h - 1));
    // What it measures, in a card above the box (below when there's no room).
    const dp = b.price - a.price;
    const pct = a.price > 0 ? (dp / a.price) * 100 : 0;
    const secs = Math.abs(b.time - a.time);
    const bars = Math.round(secs / state.interval);
    const lines = [`${fmtPct(pct)}  ${fmtSigned(dp, state.decimals)}`, state.labels.stretch(bars, secs)];
    ctx.font = FONT;
    const widest = Math.max(...lines.map((l) => ctx.measureText(l).width));
    const cardW = widest + 20;
    const cardH = 16 * lines.length + 12;
    const cx = Math.min(Math.max(x + w / 2 - cardW / 2, map.plot.x + 4), map.plot.x + map.plot.w - cardW - 4);
    const above = y - cardH - 8 >= map.plot.y;
    const cy = above ? y - cardH - 8 : y + h + 8;
    this.card(cx, cy, cardW, cardH, state.colors);
    ctx.textBaseline = "middle";
    ctx.textAlign = "center";
    lines.forEach((line, i) => {
      ctx.fillStyle = i === 0 ? rgba(color, 1) : rgba(state.colors.ink, 0.6);
      ctx.fillText(line, cx + cardW / 2, cy + 6 + 8 + 16 * i);
    });
  }

  /** A small translucent panel, like the page's popovers. */
  private card(x: number, y: number, w: number, h: number, colors: ChartColors): void {
    const { ctx } = this;
    ctx.beginPath();
    ctx.roundRect(x, y, w, h, 8);
    ctx.fillStyle = colors.dark ? "rgba(36, 38, 46, 0.92)" : "rgba(255, 255, 255, 0.94)";
    ctx.fill();
    ctx.strokeStyle = rgba(colors.ink, 0.12);
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  private handles(d: Drawing, map: Mapping, colors: ChartColors): void {
    const { ctx } = this;
    const ends = d.kind === "level" ? [] : [d.a, d.b];
    for (const end of ends) {
      ctx.beginPath();
      ctx.arc(map.x(end.time), map.y(end.price), HANDLE_R, 0, Math.PI * 2);
      ctx.fillStyle = colors.dark ? "rgb(32, 34, 41)" : "#fff";
      ctx.fill();
      ctx.strokeStyle = rgba(colors.glow, 1);
      ctx.lineWidth = 1.5;
      ctx.stroke();
    }
  }

  private crosshair(frame: LivelineFrame, map: Mapping, state: OverlayState): void {
    const { ctx } = this;
    const { pointer, snapped } = state;
    if (!state.crosshair || !pointer || !snapped || !inPlot(pointer, map.plot)) return;
    const { plot } = map;
    const x = frame.hover ? map.x(frame.hover.candle.time + state.interval / 2) : pointer.x;
    const y = map.y(snapped.price);
    ctx.setLineDash([3, 3]);
    this.line(x, plot.y, x, plot.y + plot.h, rgba(state.colors.ink, 0.3), 1);
    this.line(plot.x, y, plot.x + plot.w, y, rgba(state.colors.ink, 0.3), 1);
    ctx.setLineDash([]);
    if (snappedToCandle(frame, snapped)) {
      ctx.beginPath();
      ctx.arc(x, y, 3, 0, Math.PI * 2);
      ctx.fillStyle = rgba(state.colors.ink, 0.9);
      ctx.fill();
    }
    this.priceTag(y, fmtPrice(snapped.price, state.decimals), state.colors.ink, plot);
    if (frame.hover) this.timeTag(x, state.labels.time(frame.hover.candle.time), state.colors.ink, plot);
  }

  /** A filled tag on the price axis, inverted like a system tooltip. */
  private priceTag(y: number, text: string, color: Rgb, plot: Plot): void {
    const { ctx } = this;
    ctx.font = FONT;
    const w = ctx.measureText(text).width + 12;
    const x = plot.x + plot.w + 3;
    this.tag(x, y - TAG_H / 2, Math.min(w, this.w - x - 2), text, color);
  }

  private timeTag(x: number, text: string, color: Rgb, plot: Plot): void {
    const { ctx } = this;
    ctx.font = FONT;
    const w = ctx.measureText(text).width + 12;
    const left = Math.min(Math.max(x - w / 2, plot.x), plot.x + plot.w - w);
    this.tag(left, plot.y + plot.h + 5, w, text, color);
  }

  private tag(x: number, y: number, w: number, text: string, color: Rgb): void {
    const { ctx } = this;
    ctx.beginPath();
    ctx.roundRect(x, y, w, TAG_H, 5);
    ctx.fillStyle = rgba(color, 0.92);
    ctx.fill();
    ctx.fillStyle = this.onColor(color);
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(text, x + w / 2, y + TAG_H / 2 + 0.5);
  }

  /** Text that reads on a tag of `color`: dark ink on light hues, white on dark ones. */
  private onColor([r, g, b]: Rgb): string {
    const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    return luminance > 150 ? "rgb(24, 26, 34)" : "#fff";
  }
}
