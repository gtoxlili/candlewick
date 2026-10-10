import { useImperativeHandle, useRef, type Ref } from "react";
import { useTranslation } from "react-i18next";
import type { CandlePoint } from "liveline";
import { cn } from "cn";

import { fmtPct, fmtPrice } from "@/lib/format";
import type { Rgb } from "./palette";

/** An indicator's reading beside the candle's figures. */
export interface Reading {
  id: string;
  label: string;
  color: Rgb;
  value: number | null;
}

export interface HoverView {
  /** The pointer, in the chart's CSS pixels. */
  x: number;
  y: number;
  /** The chart's size, to keep the card inside it. */
  width: number;
  height: number;
  candle: CandlePoint;
  /** The candle's start, as a label. */
  time: string;
  readings: Reading[];
}

export interface HoverCardHandle {
  /** Moves the card to the hovered candle, or hides it with null. Called every frame. */
  update(view: HoverView | null): void;
}

/** Distance from the pointer to the card's near corner. */
const OFFSET = 14;
const MARGIN = 6;

/**
 * The hovered candle's figures in a small card that follows the pointer:
 * its start, how it closed against its open, open, high, low and close,
 * its range, and each indicator's value there. Updated outside React, as
 * the chart draws sixty frames a second.
 */
export function HoverCard(props: { decimals: number; readings: Reading[]; ref: Ref<HoverCardHandle> }) {
  const { t } = useTranslation();
  const root = useRef<HTMLDivElement>(null);
  const last = useRef<string>("");

  useImperativeHandle(props.ref, () => ({
    update(view) {
      const el = root.current;
      if (!el) return;
      if (!view) {
        if (el.style.opacity !== "0") el.style.opacity = "0";
        last.current = "";
        return;
      }
      const { candle, readings } = view;
      const key = `${candle.time}:${candle.close}:${readings.map((r) => r.value).join()}`;
      if (key !== last.current) {
        last.current = key;
        const field = (name: string) => el.querySelector<HTMLElement>(`[data-field="${name}"]`);
        const pct = candle.open > 0 ? ((candle.close - candle.open) / candle.open) * 100 : 0;
        const dir = Math.round(pct * 100);
        const change = field("change");
        if (change) {
          change.textContent = fmtPct(pct);
          change.className = cn("font-medium", dir > 0 && "text-up", dir < 0 && "text-down", dir === 0 && "text-muted-foreground");
        }
        field("time")!.textContent = view.time;
        for (const name of ["open", "high", "low", "close"] as const) {
          field(name)!.textContent = fmtPrice(candle[name], props.decimals);
        }
        const close = field("close");
        if (close) close.className = cn("text-right", dir > 0 && "text-up", dir < 0 && "text-down");
        field("range")!.textContent = t("chart.amplitude", {
          value: candle.low > 0 ? (((candle.high - candle.low) / candle.low) * 100).toFixed(2) : "0.00",
        });
        for (const reading of readings) {
          const value = el.querySelector<HTMLElement>(`[data-reading="${reading.id}"]`);
          if (value) value.textContent = reading.value === null ? "—" : fmtPrice(reading.value, props.decimals);
        }
      }
      // Beside the pointer, flipped to stay inside the chart.
      const w = el.offsetWidth;
      const h = el.offsetHeight;
      let x = view.x + OFFSET;
      if (x + w > view.width - MARGIN) x = view.x - OFFSET - w;
      let y = view.y + OFFSET;
      if (y + h > view.height - MARGIN) y = view.y - OFFSET - h;
      el.style.transform = `translate(${Math.max(MARGIN, x)}px, ${Math.max(MARGIN, y)}px)`;
      if (el.style.opacity !== "1") el.style.opacity = "1";
    },
  }));

  return (
    <div
      ref={root}
      aria-hidden
      className="pointer-events-none absolute top-0 left-0 z-10 w-max min-w-44 rounded-xl border bg-popover/90 px-3 py-2 text-xs text-popover-foreground shadow-[0_4px_16px_rgb(0_0_0/0.1),0_1px_3px_rgb(0_0_0/0.08)] backdrop-blur-xl transition-opacity duration-150 tabular will-change-transform"
      style={{ opacity: 0 }}
    >
      <div className="flex items-baseline justify-between gap-4">
        <span data-field="time" className="text-muted-foreground" />
        <span data-field="change" className="font-medium" />
      </div>
      <dl className="mt-1.5 grid grid-cols-[auto_1fr_auto_1fr] gap-x-2.5 gap-y-0.5">
        <dt className="text-muted-foreground">{t("chart.ohlc.open")}</dt>
        <dd data-field="open" className="text-right" />
        <dt className="text-muted-foreground">{t("chart.ohlc.high")}</dt>
        <dd data-field="high" className="text-right" />
        <dt className="text-muted-foreground">{t("chart.ohlc.low")}</dt>
        <dd data-field="low" className="text-right" />
        <dt className="text-muted-foreground">{t("chart.ohlc.close")}</dt>
        <dd data-field="close" className="text-right" />
      </dl>
      <p data-field="range" className="mt-1.5 text-2xs text-muted-foreground" />
      {props.readings.length > 0 && (
        <dl className="mt-1.5 grid grid-cols-[auto_1fr] gap-x-2.5 gap-y-0.5 border-t pt-1.5">
          {props.readings.map((reading) => (
            <div key={reading.id} className="contents">
              <dt className="flex items-center gap-1.5 text-muted-foreground">
                <span className="size-1.5 rounded-full" style={{ background: `rgb(${reading.color.join(" ")})` }} />
                {reading.label}
              </dt>
              <dd data-reading={reading.id} className="text-right" />
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

export interface LegendHandle {
  /** Writes each indicator's value; null reads as a dash. */
  update(readings: Reading[]): void;
}

/**
 * Indicators switched on, each with its value at the hovered candle (or
 * the latest), in the chart's top-left corner. React lays the row out
 * from the list of indicators; the values are written every frame.
 */
export function IndicatorLegend(props: { readings: Reading[]; decimals: number; ref: Ref<LegendHandle> }) {
  const root = useRef<HTMLDivElement>(null);
  useImperativeHandle(props.ref, () => ({
    update(readings) {
      const el = root.current;
      if (!el) return;
      for (const reading of readings) {
        const value = el.querySelector<HTMLElement>(`[data-reading="${reading.id}"]`);
        if (value) value.textContent = reading.value === null ? "—" : fmtPrice(reading.value, props.decimals);
      }
    },
  }));
  if (props.readings.length === 0) return null;
  return (
    <div ref={root} className="pointer-events-none absolute top-2.5 left-4 flex flex-wrap gap-x-3 gap-y-1 text-2xs tabular">
      {props.readings.map((reading) => (
        <span key={reading.id} className="flex items-center gap-1.5">
          <span className="size-1.5 rounded-full" style={{ background: `rgb(${reading.color.join(" ")})` }} />
          <span className="text-muted-foreground">{reading.label}</span>
          <span data-reading={reading.id} className="text-foreground" />
        </span>
      ))}
    </div>
  );
}
