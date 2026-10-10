// Between the chart's coordinates and the canvas: mapping a frame, snapping
// the pointer to candles, and finding the drawing under it.

import type { LivelineFrame } from "liveline";

import type { Anchor, Drawing } from "./model";
import { FIB_LEVELS } from "./model";

export interface Point {
  x: number;
  y: number;
}

export interface Plot {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Time and price to pixels and back, for one frame. */
export interface Mapping {
  x(time: number): number;
  y(price: number): number;
  time(x: number): number;
  price(y: number): number;
  plot: Plot;
}

export function mapping(frame: LivelineFrame): Mapping {
  const { left, right, min, max, plotX, plotY, plotW, plotH } = frame;
  const span = right - left || 1;
  const range = max - min || 1e-9;
  return {
    x: (time) => plotX + ((time - left) / span) * plotW,
    y: (price) => plotY + (1 - (price - min) / range) * plotH,
    time: (x) => left + ((x - plotX) / plotW) * span,
    price: (y) => min + (1 - (y - plotY) / plotH) * range,
    plot: { x: plotX, y: plotY, w: plotW, h: plotH },
  };
}

export const inPlot = (p: Point, plot: Plot): boolean =>
  p.x >= plot.x && p.x <= plot.x + plot.w && p.y >= plot.y && p.y <= plot.y + plot.h;

/** Pixels within which the pointer sticks to a candle's open, high, low or close. */
const SNAP_PX = 8;

/**
 * Where a point lands in the chart: on the hovered candle's center and,
 * when close enough, on one of its prices (`free` turns the latter off).
 */
export function snap(frame: LivelineFrame, map: Mapping, p: Point, interval: number, free: boolean): Anchor {
  const candle = frame.hover?.candle;
  const time = candle ? candle.time + interval / 2 : map.time(p.x);
  let price = map.price(p.y);
  if (candle && !free) {
    let best = SNAP_PX;
    for (const level of [candle.open, candle.high, candle.low, candle.close]) {
      const d = Math.abs(map.y(level) - p.y);
      if (d < best) {
        best = d;
        price = level;
      }
    }
  }
  return { time, price };
}

/** Whether `anchor` sits exactly on one of the candle's prices (it snapped). */
export function snappedToCandle(frame: LivelineFrame, anchor: Anchor): boolean {
  const c = frame.hover?.candle;
  return !!c && [c.open, c.high, c.low, c.close].includes(anchor.price);
}

/** A drawing's body, or one of its ends by index. */
export type HitPart = "body" | 0 | 1;

export interface Hit {
  id: string;
  part: HitPart;
}

const HANDLE_PX = 7;
const LINE_PX = 5;

function distToSegment(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2));
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy));
}

/** Where a fib's level lines run: from the earlier end to the plot's right edge. */
export function fibSpan(a: Point, b: Point, plot: Plot): [number, number] {
  return [Math.min(a.x, b.x), plot.x + plot.w];
}

/** The topmost drawing under `p`, ends before bodies, latest drawn first. */
export function hitTest(drawings: readonly Drawing[], map: Mapping, p: Point): Hit | null {
  for (let i = drawings.length - 1; i >= 0; i--) {
    const d = drawings[i];
    if (d.kind === "level") {
      if (Math.abs(map.y(d.price) - p.y) <= LINE_PX) return { id: d.id, part: "body" };
      continue;
    }
    const a = { x: map.x(d.a.time), y: map.y(d.a.price) };
    const b = { x: map.x(d.b.time), y: map.y(d.b.price) };
    if (Math.hypot(a.x - p.x, a.y - p.y) <= HANDLE_PX) return { id: d.id, part: 0 };
    if (Math.hypot(b.x - p.x, b.y - p.y) <= HANDLE_PX) return { id: d.id, part: 1 };
    switch (d.kind) {
      case "trend":
        if (distToSegment(p, a, b) <= LINE_PX) return { id: d.id, part: "body" };
        break;
      case "fib": {
        const [x0, x1] = fibSpan(a, b, map.plot);
        const onLevel = FIB_LEVELS.some((level) => {
          const y = a.y + (b.y - a.y) * level;
          return p.x >= x0 - LINE_PX && p.x <= x1 && Math.abs(y - p.y) <= LINE_PX;
        });
        if (onLevel || distToSegment(p, a, b) <= LINE_PX) return { id: d.id, part: "body" };
        break;
      }
      case "measure": {
        const inside =
          p.x >= Math.min(a.x, b.x) - LINE_PX &&
          p.x <= Math.max(a.x, b.x) + LINE_PX &&
          p.y >= Math.min(a.y, b.y) - LINE_PX &&
          p.y <= Math.max(a.y, b.y) + LINE_PX;
        if (inside) return { id: d.id, part: "body" };
        break;
      }
    }
  }
  return null;
}
