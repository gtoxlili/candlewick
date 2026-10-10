import { useSyncExternalStore, type ComponentProps } from "react";
import { useTranslation } from "react-i18next";
import { Eraser, Minus, MousePointer2, Ruler, Sigma, TrendingUp, Undo2 } from "lucide-react";
import { Popover } from "radix-ui";
import { cn } from "cn";

import { Switch } from "@/components/ui/switch";
import type { Analysis } from "./analysis/analysis";
import { MAX_PERIOD, MIN_PERIOD, indicatorLabel } from "./analysis/indicators";
import type { Tool } from "./analysis/model";
import { indicatorColor } from "./analysis/overlay";
import type { ChartColors } from "./palette";

/** Fibonacci retracement: levels of shortening reach. Drawn in Lucide's idiom. */
function FibIcon(props: ComponentProps<"svg">) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" {...props}>
      <path d="M4 5h16M4 10h11M4 14.5h7M4 19h16" />
    </svg>
  );
}

const TOOLS = [
  { value: "cursor", Icon: MousePointer2, key: "V" },
  { value: "level", Icon: Minus, key: "H" },
  { value: "trend", Icon: TrendingUp, key: "T" },
  { value: "fib", Icon: FibIcon, key: "F" },
  { value: "measure", Icon: Ruler, key: "M" },
] as const satisfies readonly { value: Tool; Icon: unknown; key: string }[];

/** A toggle in the pill, lit when pressed, as the chart type toggle draws it. */
export const toggleClass = (pressed: boolean) =>
  cn(
    "flex h-6 w-7 items-center justify-center rounded-full transition-all duration-200 outline-none focus-visible:ring-2 focus-visible:ring-ring",
    pressed
      ? "bg-white text-foreground shadow-[0_1px_2px_rgb(0_0_0/0.12)] dark:bg-white/16 dark:shadow-none"
      : "text-muted-foreground hover:text-foreground",
  );

/**
 * The analysis controls of the chart's header: the drawing tools, the
 * indicators menu, and, while there is something to undo or clear, the
 * buttons for that.
 */
export function AnalysisBar(props: { analysis: Analysis; colors: ChartColors }) {
  const { t } = useTranslation();
  const { analysis } = props;
  const snapshot = useSyncExternalStore(analysis.subscribe, analysis.snapshot);
  const anyOn = snapshot.indicators.some((i) => i.on);
  return (
    <div className="flex items-center gap-1">
      {snapshot.canUndo && (
        <button
          type="button"
          title={t("chart.undo")}
          aria-label={t("chart.undo")}
          onClick={() => analysis.undo()}
          className="flex size-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-fill hover:text-foreground"
        >
          <Undo2 className="size-3.5" />
        </button>
      )}
      {snapshot.drawings > 0 && (
        <button
          type="button"
          title={t("chart.clearDrawings")}
          aria-label={t("chart.clearDrawings")}
          onClick={() => analysis.clearAll()}
          className="flex size-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-fill hover:text-destructive"
        >
          <Eraser className="size-3.5" />
        </button>
      )}
      <div role="toolbar" aria-label={t("chart.tools")} className="pill flex items-center p-0.5">
        {TOOLS.map(({ value, Icon, key }) => {
          const label = t(`chart.tool.${value}`);
          const pressed = snapshot.tool === value;
          return (
            <button
              key={value}
              type="button"
              aria-pressed={pressed}
              title={`${label}  ⌥${key}`}
              aria-label={label}
              onClick={() => analysis.setTool(value)}
              className={toggleClass(pressed)}
            >
              <Icon className="size-3.5" />
            </button>
          );
        })}
        <span aria-hidden className="mx-1 h-3.5 w-px bg-border" />
        <Popover.Root>
          <Popover.Trigger asChild>
            <button type="button" aria-pressed={anyOn} title={t("chart.indicators")} aria-label={t("chart.indicators")} className={toggleClass(anyOn)}>
              <Sigma className="size-3.5" />
            </button>
          </Popover.Trigger>
          <Popover.Portal>
            <Popover.Content
              align="end"
              sideOffset={8}
              collisionPadding={8}
              className="z-20 w-60 rounded-xl border bg-popover p-1.5 text-popover-foreground shadow-lg outline-none backdrop-blur-xl select-none animate-in fade-in zoom-in-95 duration-150"
            >
              <p className="px-2 pt-1 pb-1.5 text-2xs font-medium text-muted-foreground">{t("chart.indicators")}</p>
              {snapshot.indicators.map((spec) => (
                <label key={spec.id} className="flex h-8 cursor-default items-center gap-2 rounded-lg px-2 hover:bg-fill">
                  <span
                    className="size-2 shrink-0 rounded-full"
                    style={{ background: `rgb(${indicatorColor(spec, props.colors).join(" ")})` }}
                  />
                  <span className="min-w-0 flex-1 truncate text-xs">{t(`chart.indicator.${spec.kind}`)}</span>
                  <input
                    type="number"
                    inputMode="numeric"
                    min={MIN_PERIOD}
                    max={MAX_PERIOD}
                    value={spec.period}
                    aria-label={`${indicatorLabel(spec)} · ${t("chart.period")}`}
                    onChange={(e) => {
                      const period = Number(e.target.value);
                      if (Number.isFinite(period) && period >= MIN_PERIOD) analysis.setIndicator(spec.id, { period });
                    }}
                    className="h-6 w-11 appearance-none rounded-md border border-input bg-transparent px-1.5 text-center text-xs tabular outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                  />
                  <Switch size="sm" checked={spec.on} onCheckedChange={(on) => analysis.setIndicator(spec.id, { on })} />
                </label>
              ))}
            </Popover.Content>
          </Popover.Portal>
        </Popover.Root>
      </div>
    </div>
  );
}
