// The analysis layer's state and gestures: which tool is active, the
// drawings of the instrument on screen (selected, hovered, dragged, or
// half drawn), the indicators switched on and their values. The chart
// hands it pointer events and the latest frame; the overlay reads it
// every frame; React subscribes for the toolbar.

import type { CandlePoint, LivelineFrame } from "liveline";

import { hitTest, inPlot, mapping, snap, type Hit, type HitPart, type Point } from "./geometry";
import {
  compute,
  loadIndicators,
  storeIndicators,
  type IndicatorSeries,
  type IndicatorSpec,
  MAX_PERIOD,
  MIN_PERIOD,
} from "./indicators";
import {
  anchorsNeeded,
  loadDrawings,
  makeDrawing,
  moved,
  storeDrawings,
  withAnchor,
  type Anchor,
  type Drawing,
  type DrawingKind,
  type Tool,
} from "./model";

/** What the toolbar shows; a new object whenever any of it changes. */
export interface AnalysisSnapshot {
  tool: Tool;
  selected: string | null;
  drawings: number;
  indicators: IndicatorSpec[];
  canUndo: boolean;
  /** What the pointer is over, for the cursor. */
  hovered: HitPart | null;
  dragging: boolean;
  drafting: boolean;
}

/** What a press became. */
export type Press = "draw" | "drag" | "pan";

interface Drag {
  id: string;
  part: HitPart;
  /** Where the press landed, unsnapped. */
  from: Anchor;
  original: Drawing;
}

const UNDO_DEPTH = 20;

export class Analysis {
  tool: Tool = "cursor";
  drawings: Drawing[] = [];
  selected: string | null = null;
  hovered: Hit | null = null;
  draft: { kind: DrawingKind; points: Anchor[] } | null = null;
  pointer: Point | null = null;
  snapped: Anchor | null = null;
  indicators: IndicatorSpec[] = loadIndicators();
  series: IndicatorSeries[] = [];
  candles: readonly CandlePoint[] = [];
  private instrument: string | null = null;
  private drag: Drag | null = null;
  private history: Drawing[][] = [];
  private listeners = new Set<() => void>();
  private cached: AnalysisSnapshot | null = null;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  snapshot = (): AnalysisSnapshot =>
    (this.cached ??= {
      tool: this.tool,
      selected: this.selected,
      drawings: this.drawings.length,
      indicators: this.indicators,
      canUndo: this.history.length > 0,
      hovered: this.hovered?.part ?? null,
      dragging: this.drag !== null,
      drafting: this.draft !== null,
    });

  private changed(): void {
    this.cached = null;
    for (const listener of this.listeners) listener();
  }

  /** Another instrument: its own drawings, nothing selected or half drawn. */
  setInstrument(id: string | null): void {
    this.instrument = id;
    this.drawings = id ? loadDrawings(id) : [];
    this.selected = null;
    this.hovered = null;
    this.draft = null;
    this.drag = null;
    this.history = [];
    this.changed();
  }

  /** Picks a tool; picking the active one again puts the cursor back. */
  setTool(tool: Tool): void {
    const next = tool === this.tool && tool !== "cursor" ? "cursor" : tool;
    this.tool = next;
    this.draft = null;
    if (next !== "cursor") this.selected = null;
    this.changed();
  }

  setIndicator(id: string, patch: Partial<Pick<IndicatorSpec, "on" | "period">>): void {
    this.indicators = this.indicators.map((spec) => {
      if (spec.id !== id) return spec;
      const period = Math.max(MIN_PERIOD, Math.min(MAX_PERIOD, Math.round(patch.period ?? spec.period)));
      return { ...spec, ...patch, period };
    });
    storeIndicators(this.indicators);
    this.recompute();
    this.changed();
  }

  /** The candles on screen (closed ones and the forming one), oldest first. */
  setCandles(candles: readonly CandlePoint[]): void {
    this.candles = candles;
    this.recompute();
  }

  private recompute(): void {
    this.series = this.indicators.filter((spec) => spec.on).map((spec) => compute(spec, this.candles));
  }

  /** The position of the candle opening at `time`, or null. */
  indexOf(time: number): number | null {
    const { candles } = this;
    let lo = 0;
    let hi = candles.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const t = candles[mid].time;
      if (t === time) return mid;
      if (t < time) lo = mid + 1;
      else hi = mid - 1;
    }
    return null;
  }

  /** Each indicator switched on, with its value at candle `index` (the latest when null). */
  valuesAt(index: number | null): { spec: IndicatorSpec; value: number | null }[] {
    return this.series.map((series) => {
      const i = index ?? series.values.length - 1;
      const value = i >= 0 && i < series.values.length ? series.values[i] : NaN;
      return { spec: series.spec, value: Number.isFinite(value) ? value : null };
    });
  }

  /**
   * The primary button went down at `p` (CSS pixels in the chart). With a
   * drawing tool it places a point; on a drawing it starts a drag; else the
   * chart may pan. `free` (Alt) turns snapping off.
   */
  pointerDown(p: Point, frame: LivelineFrame | null, interval: number, free: boolean): Press {
    if (!frame) return "pan";
    const map = mapping(frame);
    if (!inPlot(p, map.plot)) return "pan";
    const anchor = snap(frame, map, p, interval, free);
    if (this.tool !== "cursor") {
      const kind = this.tool;
      const points = [...(this.draft?.points ?? []), anchor];
      if (points.length >= anchorsNeeded(kind)) {
        this.drawings = [...this.drawings, makeDrawing(kind, points)];
        this.draft = null;
        this.tool = "cursor";
        this.persist();
      } else {
        this.draft = { kind, points };
      }
      this.changed();
      return "draw";
    }
    const hit = hitTest(this.drawings, map, p);
    if (hit) {
      const original = this.drawings.find((d) => d.id === hit.id)!;
      this.selected = hit.id;
      this.drag = { id: hit.id, part: hit.part, from: { time: map.time(p.x), price: map.price(p.y) }, original };
      this.changed();
      return "drag";
    }
    if (this.selected !== null) {
      this.selected = null;
      this.changed();
    }
    return "pan";
  }

  pointerMove(p: Point, frame: LivelineFrame | null, interval: number, free: boolean): void {
    this.pointer = p;
    if (!frame) return;
    const map = mapping(frame);
    this.snapped = inPlot(p, map.plot) ? snap(frame, map, p, interval, free) : null;
    const drag = this.drag;
    if (drag) {
      const here = { time: map.time(p.x), price: map.price(p.y) };
      const next =
        drag.part === "body"
          ? moved(drag.original, here.time - drag.from.time, here.price - drag.from.price)
          : withAnchor(drag.original, drag.part, this.snapped ?? here);
      this.drawings = this.drawings.map((d) => (d.id === drag.id ? next : d));
      return;
    }
    const hit = this.tool === "cursor" && this.snapped ? hitTest(this.drawings, map, p) : null;
    if (hit?.id !== this.hovered?.id || hit?.part !== this.hovered?.part) {
      this.hovered = hit;
      this.changed();
    }
  }

  pointerUp(): void {
    if (!this.drag) return;
    this.drag = null;
    this.persist();
    this.changed();
  }

  pointerLeave(): void {
    this.pointer = null;
    this.snapped = null;
    if (this.hovered) {
      this.hovered = null;
      this.changed();
    }
  }

  /** Escape: drops a half-drawn shape, else the selection, else the tool. Returns whether it did anything. */
  cancel(): boolean {
    if (this.draft) {
      this.draft = null;
      this.tool = "cursor";
    } else if (this.selected !== null) {
      this.selected = null;
    } else if (this.tool !== "cursor") {
      this.tool = "cursor";
    } else {
      return false;
    }
    this.changed();
    return true;
  }

  removeSelected(): void {
    if (this.selected === null) return;
    this.remember();
    this.drawings = this.drawings.filter((d) => d.id !== this.selected);
    this.selected = null;
    this.hovered = null;
    this.persist();
    this.changed();
  }

  clearAll(): void {
    if (this.drawings.length === 0) return;
    this.remember();
    this.drawings = [];
    this.selected = null;
    this.hovered = null;
    this.persist();
    this.changed();
  }

  /** Brings back what the last removal took. */
  undo(): void {
    const previous = this.history.pop();
    if (!previous) return;
    this.drawings = previous;
    this.persist();
    this.changed();
  }

  private remember(): void {
    this.history = [...this.history.slice(-(UNDO_DEPTH - 1)), this.drawings];
  }

  private persist(): void {
    if (this.instrument) storeDrawings(this.instrument, this.drawings);
  }
}
