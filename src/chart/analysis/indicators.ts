// Overlay indicators computed from the candles on hand: moving averages
// and Bollinger Bands. Settings are the viewer's, kept across instruments.

import type { CandlePoint } from "liveline";

import { load, store } from "@/lib/prefs";

export type IndicatorKind = "sma" | "ema" | "boll";

export interface IndicatorSpec {
  id: string;
  kind: IndicatorKind;
  period: number;
  on: boolean;
}

/** The common set, off until switched on. */
export const DEFAULT_INDICATORS: IndicatorSpec[] = [
  { id: "ma1", kind: "sma", period: 7, on: false },
  { id: "ma2", kind: "sma", period: 25, on: false },
  { id: "ma3", kind: "sma", period: 99, on: false },
  { id: "ema", kind: "ema", period: 12, on: false },
  { id: "boll", kind: "boll", period: 20, on: false },
];

/** Standard deviations the bands sit from the middle. */
export const BOLL_MULT = 2;
export const MIN_PERIOD = 2;
export const MAX_PERIOD = 500;

/** "MA7", "EMA12", "BOLL(20,2)" */
export function indicatorLabel(spec: IndicatorSpec): string {
  switch (spec.kind) {
    case "sma":
      return `MA${spec.period}`;
    case "ema":
      return `EMA${spec.period}`;
    case "boll":
      return `BOLL(${spec.period},${BOLL_MULT})`;
  }
}

/** An indicator's values, one per candle (NaN before it has enough history). */
export interface IndicatorSeries {
  spec: IndicatorSpec;
  values: Float64Array;
  /** Bollinger Bands only. */
  upper?: Float64Array;
  lower?: Float64Array;
}

/** `candles` oldest first, the forming one last. */
export function compute(spec: IndicatorSpec, candles: readonly CandlePoint[]): IndicatorSeries {
  const n = candles.length;
  const period = Math.max(MIN_PERIOD, Math.min(MAX_PERIOD, Math.round(spec.period)));
  const values = new Float64Array(n).fill(NaN);
  switch (spec.kind) {
    case "sma": {
      let sum = 0;
      for (let i = 0; i < n; i++) {
        sum += candles[i].close;
        if (i >= period) sum -= candles[i - period].close;
        if (i >= period - 1) values[i] = sum / period;
      }
      return { spec, values };
    }
    case "ema": {
      const k = 2 / (period + 1);
      let sum = 0;
      for (let i = 0; i < n; i++) {
        const close = candles[i].close;
        if (i < period) {
          sum += close;
          if (i === period - 1) values[i] = sum / period;
        } else {
          values[i] = close * k + values[i - 1] * (1 - k);
        }
      }
      return { spec, values };
    }
    case "boll": {
      const upper = new Float64Array(n).fill(NaN);
      const lower = new Float64Array(n).fill(NaN);
      let sum = 0;
      let squares = 0;
      for (let i = 0; i < n; i++) {
        const close = candles[i].close;
        sum += close;
        squares += close * close;
        if (i >= period) {
          const gone = candles[i - period].close;
          sum -= gone;
          squares -= gone * gone;
        }
        if (i >= period - 1) {
          const mean = sum / period;
          const variance = Math.max(0, squares / period - mean * mean);
          const dev = Math.sqrt(variance) * BOLL_MULT;
          values[i] = mean;
          upper[i] = mean + dev;
          lower[i] = mean - dev;
        }
      }
      return { spec, values, upper, lower };
    }
  }
}

const KEY = "chart.indicators";

function isSpec(value: unknown): value is IndicatorSpec {
  if (!value || typeof value !== "object") return false;
  const s = value as Partial<IndicatorSpec>;
  return (
    typeof s.id === "string" &&
    (s.kind === "sma" || s.kind === "ema" || s.kind === "boll") &&
    typeof s.period === "number" &&
    Number.isFinite(s.period) &&
    typeof s.on === "boolean"
  );
}

export function loadIndicators(): IndicatorSpec[] {
  return load(
    KEY,
    (raw) => {
      const value: unknown = JSON.parse(raw);
      const specs = Array.isArray(value) ? value.filter(isSpec) : [];
      return specs.length === DEFAULT_INDICATORS.length ? specs : undefined;
    },
    DEFAULT_INDICATORS,
  );
}

export function storeIndicators(specs: IndicatorSpec[]): void {
  store(KEY, JSON.stringify(specs));
}
