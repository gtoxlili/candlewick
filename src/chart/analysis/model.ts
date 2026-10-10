// What someone draws on the chart, in the chart's own coordinates (epoch
// seconds and price), so a drawing stays put across intervals, zooms and
// pans. Kept per instrument in local storage.

import { load, store } from "@/lib/prefs";

export interface Anchor {
  /** Epoch seconds, at a candle's center once snapped. */
  time: number;
  price: number;
}

export type Drawing =
  /** A horizontal price level. */
  | { id: string; kind: "level"; price: number }
  /** A straight line between two points. */
  | { id: string; kind: "trend"; a: Anchor; b: Anchor }
  /** Fibonacci retracement levels between a swing's two ends. */
  | { id: string; kind: "fib"; a: Anchor; b: Anchor }
  /** A measured stretch: price and time between two points. */
  | { id: string; kind: "measure"; a: Anchor; b: Anchor };

export type DrawingKind = Drawing["kind"];

/** What a click on the chart does: pans and selects, or starts a drawing. */
export type Tool = "cursor" | DrawingKind;

/** Retracement levels, as every charting tool lists them. */
export const FIB_LEVELS = [0, 0.236, 0.382, 0.5, 0.618, 0.786, 1] as const;

/** Points a drawing of `kind` takes before it is complete. */
export const anchorsNeeded = (kind: DrawingKind): number => (kind === "level" ? 1 : 2);

let counter = 0;

/** A drawing from the points clicked, in order. */
export function makeDrawing(kind: DrawingKind, points: Anchor[]): Drawing {
  const id = `${Date.now().toString(36)}${(counter++).toString(36)}`;
  const [a, b] = points;
  if (kind === "level") return { id, kind, price: a.price };
  return { id, kind, a, b: b ?? a };
}

/** The movable ends of a drawing; a level has none, it moves as a whole. */
export function anchorsOf(drawing: Drawing): Anchor[] {
  return drawing.kind === "level" ? [] : [drawing.a, drawing.b];
}

/** The drawing shifted by `dt` seconds and `dp` in price. */
export function moved(drawing: Drawing, dt: number, dp: number): Drawing {
  if (drawing.kind === "level") return { ...drawing, price: drawing.price + dp };
  const shift = (p: Anchor): Anchor => ({ time: p.time + dt, price: p.price + dp });
  return { ...drawing, a: shift(drawing.a), b: shift(drawing.b) };
}

/** The drawing with end `index` (0 or 1) at `anchor`. */
export function withAnchor(drawing: Drawing, index: number, anchor: Anchor): Drawing {
  if (drawing.kind === "level") return { ...drawing, price: anchor.price };
  return index === 0 ? { ...drawing, a: anchor } : { ...drawing, b: anchor };
}

const KEY = (instrument: string) => `chart.drawings.${instrument}`;

function isDrawing(value: unknown): value is Drawing {
  if (!value || typeof value !== "object") return false;
  const d = value as Partial<Drawing>;
  if (typeof d.id !== "string") return false;
  if (d.kind === "level") return typeof d.price === "number";
  const anchor = (p: unknown) =>
    !!p && typeof p === "object" && typeof (p as Anchor).time === "number" && typeof (p as Anchor).price === "number";
  return (d.kind === "trend" || d.kind === "fib" || d.kind === "measure") && anchor(d.a) && anchor(d.b);
}

export function loadDrawings(instrument: string): Drawing[] {
  return load(
    KEY(instrument),
    (raw) => {
      const value: unknown = JSON.parse(raw);
      return Array.isArray(value) ? value.filter(isDrawing) : undefined;
    },
    [] as Drawing[],
  );
}

export function storeDrawings(instrument: string, drawings: Drawing[]): void {
  store(KEY(instrument), JSON.stringify(drawings));
}
