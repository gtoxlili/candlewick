// Which stretch of time the chart shows. Gestures move targets on one object
// that Liveline reads every frame (its `view` prop), so panning and zooming
// never go through React; only what the page itself shows is reported back:
// whether the view follows the live edge, and the zoom to remember.

import type { LivelineView } from "liveline";

export const DEFAULT_BARS = 60;
const MIN_BARS = 15;
const MAX_BARS = 360;
/** How quickly a flung drag slows down (time constant). */
const GLIDE_MS = 325;
/** Slower than this (px/ms) a released drag just stops. */
const GLIDE_MIN = 0.05;

export interface ViewportEvents {
  live(live: boolean): void;
  /** The view reached within a screen of the oldest candle. */
  older(): void;
  zoom(bars: number): void;
}

export const clampBars = (bars: number) => Math.min(MAX_BARS, Math.max(MIN_BARS, bars));

export class Viewport {
  readonly view: LivelineView;
  private secs: number;
  private oldest = Number.POSITIVE_INFINITY;
  private exhausted = true;
  private dragFrom: number | null = null;
  private glideFrame = 0;
  private events: ViewportEvents | null = null;

  constructor(secs: number, bars: number) {
    this.secs = secs;
    this.view = { span: clampBars(bars) * secs, end: null };
  }

  listen(events: ViewportEvents | null): void {
    this.events = events;
  }

  get live(): boolean {
    return this.view.end === null;
  }

  /** New candle size (or pair): the same number of bars, back at the live edge. */
  selectInterval(secs: number): void {
    const bars = this.view.span / this.secs;
    this.secs = secs;
    this.stopGlide();
    this.view.span = clampBars(bars) * secs;
    // Pinned to the live edge until this interval's history says how far back it goes.
    this.oldest = Number.POSITIVE_INFINITY;
    this.exhausted = true;
    this.setEnd(null);
  }

  /** Where the loaded history starts, so panning stops there (or pages in more). */
  setHistory(oldest: number | null, exhausted: boolean): void {
    this.oldest = oldest ?? Number.POSITIVE_INFINITY;
    this.exhausted = exhausted;
    // The oldest candles can go (a reload, the cap): keep the view on what is left.
    if (this.view.end !== null) this.moveTo(this.view.end, false);
    this.wantOlder();
  }

  dragStart(): void {
    this.stopGlide();
    this.dragFrom = this.view.right ?? this.targetEnd();
  }

  /** `dx`: pixels moved since `dragStart`; content follows the pointer. */
  dragTo(dx: number): void {
    if (this.dragFrom === null) return;
    this.moveTo(this.dragFrom - dx * this.secsPerPx(), true);
  }

  /** `velocity`: pixels per millisecond at release. */
  dragEnd(velocity: number): void {
    this.dragFrom = null;
    if (Math.abs(velocity) >= GLIDE_MIN) this.glide(velocity);
  }

  /** Horizontal scroll (trackpad swipe); positive moves toward the present. */
  scrollBy(dx: number): void {
    this.stopGlide();
    this.moveTo(this.targetEnd() + dx * this.secsPerPx(), true);
  }

  /**
   * `factor` above 1 shows more time. At the live edge the newest candle stays
   * put; in history the time under `x` (CSS pixels in the chart) does.
   */
  zoom(factor: number, x: number): void {
    this.stopGlide();
    const span = clampBars((this.view.span * factor) / this.secs) * this.secs;
    if (span === this.view.span) return;
    const { left, right, plotX, plotW } = this.view;
    if (this.live || left === undefined || right === undefined || plotX === undefined || !plotW) {
      this.view.span = span;
    } else {
      const u = Math.min(1, Math.max(0, (x - plotX) / plotW));
      const anchor = left + u * (right - left);
      this.view.span = span;
      this.moveTo(anchor + (1 - u) * span, false);
    }
    this.events?.zoom(span / this.secs);
    this.wantOlder();
  }

  goLive(): void {
    this.stopGlide();
    this.setEnd(null);
  }

  reset(): void {
    this.view.span = DEFAULT_BARS * this.secs;
    this.events?.zoom(DEFAULT_BARS);
    this.goLive();
  }

  dispose(): void {
    this.stopGlide();
    this.events = null;
  }

  private targetEnd(): number {
    return this.view.end ?? this.liveEnd(this.view.span);
  }

  private liveEnd(span: number): number {
    return Date.now() / 1000 + span * (this.view.buffer ?? 0);
  }

  private secsPerPx(): number {
    const { left, right, plotW } = this.view;
    if (left === undefined || right === undefined || !plotW) return this.view.span / 600;
    return (right - left) / plotW;
  }

  /** Right edge, kept between the loaded history and the present. */
  private moveTo(end: number, snap: boolean): void {
    const span = this.view.span;
    const liveEnd = this.liveEnd(span);
    if (end >= liveEnd) {
      this.setEnd(null, snap);
      return;
    }
    // At least a quarter of the view stays on candles; with less history than
    // that there is nowhere to pan, so the view keeps following.
    const clamped = Math.max(end, this.oldest + span / 4);
    this.setEnd(clamped >= liveEnd ? null : clamped, snap);
    this.wantOlder();
  }

  private setEnd(end: number | null, snap = false): void {
    const wasLive = this.live;
    this.view.end = end;
    if (snap) this.view.snap = true;
    if (wasLive !== this.live) this.events?.live(this.live);
  }

  private wantOlder(): void {
    if (this.exhausted || !Number.isFinite(this.oldest)) return;
    if (this.targetEnd() - this.view.span < this.oldest + this.view.span) this.events?.older();
  }

  private glide(velocity: number): void {
    let v = velocity;
    let last = performance.now();
    const step = (now: number) => {
      const dt = now - last;
      last = now;
      v *= Math.exp(-dt / GLIDE_MS);
      const before = this.view.end;
      if (Math.abs(v) >= GLIDE_MIN / 4 && before !== null) {
        this.moveTo(this.targetEnd() - v * dt * this.secsPerPx(), true);
        // Stopped by the edge of history or reached the present.
        if (this.view.end !== before && this.view.end !== null) {
          this.glideFrame = requestAnimationFrame(step);
          return;
        }
      }
      this.glideFrame = 0;
    };
    this.glideFrame = requestAnimationFrame(step);
  }

  private stopGlide(): void {
    cancelAnimationFrame(this.glideFrame);
    this.glideFrame = 0;
  }
}
