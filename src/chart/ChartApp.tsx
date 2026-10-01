import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ChartCandlestick, ChartLine, ExternalLink } from "lucide-react";
import { setFrameRate, setTrendColors } from "liveline";
import { cn } from "cn";

import { ChangeBadge } from "@/components/ChangeBadge";
import { Picker } from "@/components/Picker";
import { PillTabs } from "@/components/PillTabs";
import { TITLE_INSET, TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import {
  api,
  instrumentId,
  instrumentLabel,
  subscribe,
  type Book,
  type ChartMode,
  type ChartSpec,
  type FeedState,
  type Instrument,
  type Interval,
  type Settings,
  type Stats,
  type Trade,
} from "@/lib/api";
import { direction, fmtCompact, fmtPct, fmtPrice, priceDecimals } from "@/lib/format";
import { Market } from "./market";
import { bidShare, OrderBook } from "./OrderBook";
import { readChartColors, sameColors, type ChartColors } from "./palette";
import { load, store } from "@/lib/prefs";
import { PriceChart, type VisibleStats } from "./PriceChart";
import { TradeList } from "./TradeList";

type Tab = "book" | "trades";

const TABS = [
  { value: "book", label: "盘口" },
  { value: "trades", label: "成交" },
] as const satisfies readonly { value: Tab; label: string }[];

/** The remembered interval if the provider offers it, else one minute, else its first. */
function pickInterval(intervals: Interval[], secs: number): Interval | null {
  return intervals.find((i) => i.secs === secs) ?? intervals.find((i) => i.secs === 60) ?? intervals[0] ?? null;
}

const parseModes = (raw: string): Record<string, ChartMode> | undefined => {
  const value: unknown = JSON.parse(raw);
  return value && typeof value === "object" ? (value as Record<string, ChartMode>) : undefined;
};

export default function ChartApp() {
  const [id, setId] = useState(() => new URLSearchParams(location.search).get("id") ?? "");
  const [settings, setSettings] = useState<Settings | null>(null);
  /** The spec of the instrument it was loaded for. */
  const [spec, setSpec] = useState<{ id: string; spec: ChartSpec } | null>(null);
  const [intervalSecs, setIntervalSecs] = useState(() =>
    load("chart.interval", (raw) => Number(raw) || undefined, 60 as number),
  );
  const [modes, setModes] = useState(() =>
    load("chart.modeByInterval", parseModes, {} as Record<string, ChartMode>),
  );
  const [tab, setTab] = useState<Tab>(() =>
    load("chart.tab", (raw) => TABS.find((t) => t.value === raw)?.value, "book" as Tab),
  );
  const [market, setMarket] = useState<Market | null>(null);
  const [visible, setVisible] = useState<VisibleStats | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [lastDirection, setLastDirection] = useState<1 | -1 | 0>(0);
  /** The book grouped each way the provider offers; null until the first arrives. */
  const [books, setBooks] = useState<Book[] | null>(null);
  /** Which of the provider's book steps to show, finest first. */
  const [bookStep, setBookStep] = useState(() => load("chart.bookStep", (raw) => Number(raw) || undefined, 0));
  /** null until the first list arrives. */
  const [trades, setTrades] = useState<Trade[] | null>(null);
  const [feed, setFeed] = useState<FeedState>("connecting");
  const [appearance, setAppearance] = useState(0);
  const [colors, setColors] = useState<ChartColors>(() => {
    const initial = readChartColors();
    setTrendColors(initial.up, initial.down);
    return initial;
  });
  const [chartKey, setChartKey] = useState(0);

  const currentInterval = useRef(intervalSecs);
  const currentTab = useRef(tab);

  const chartSpec = spec?.id === id ? spec.spec : null;
  const interval = pickInterval(chartSpec?.intervals ?? [], intervalSecs);
  const mode: ChartMode = (interval && modes[interval.secs]) ?? interval?.mode ?? "candle";
  const instrument: Instrument | undefined = settings?.watchlist.find((i) => instrumentId(i) === id);
  const volumeUnit = chartSpec?.volumeUnit ?? "";
  const priceUnit = chartSpec?.turnoverUnit ?? "";
  const decimals = priceDecimals(stats?.last ?? 0, instrument?.decimals ?? null);
  const bookSteps = chartSpec?.bookSteps ?? [];
  const step = Math.min(Math.max(bookStep, 0), Math.max(bookSteps.length - 1, 0));
  const book = books?.[Math.min(step, books.length - 1)] ?? null;
  const scheme = settings?.colorScheme ?? "greenUp";

  // Settings, the tray switching instruments, and showing the window.
  useEffect(() => {
    // The URL names the instrument the window was opened for; a switch
    // requested while this page was still loading only reached the app, so
    // read its record once listening. A switch event is always newer than that read.
    let switched = false;
    const instrumentEvents = api.onChartInstrument((next) => {
      switched = true;
      setId(next);
    });
    const stops = [subscribe(api.onSettings(setSettings)), subscribe(instrumentEvents)];
    instrumentEvents
      .then(() => api.getChartInstrument())
      .then((stored) => {
        if (stored && !switched) setId(stored);
      })
      .catch(() => {});
    api
      .getSettings()
      .then(setSettings)
      .catch(() => {});
    void api.ready();
    return () => stops.forEach((stop) => stop());
  }, []);

  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setAppearance((n) => n + 1);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  // Appearance or the red/green convention changed: recolor the chart
  // (remounted so Liveline re-resolves its palette with the new colors).
  useEffect(() => {
    document.documentElement.dataset.scheme = scheme;
    const next = readChartColors();
    if (sameColors(colors, next)) return;
    setTrendColors(next.up, next.down);
    setColors(next);
    setChartKey((k) => k + 1);
  }, [scheme, appearance, colors]);

  useEffect(() => {
    currentInterval.current = intervalSecs;
    store("chart.interval", String(intervalSecs));
  }, [intervalSecs]);

  // What the instrument's provider offers: intervals, units, link.
  useEffect(() => {
    if (!id) return;
    let current = true;
    api
      .chartSpec(id)
      .then((next) => current && setSpec({ id, spec: next }))
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [id]);

  // One feed per instrument; interval and tab changes reuse its stream.
  useEffect(() => {
    setStats(null);
    setBooks(null);
    setTrades(null);
    setLastDirection(0);
    const start = chartSpec && pickInterval(chartSpec.intervals, currentInterval.current);
    if (!start) {
      setMarket(null);
      return;
    }
    let previousLast: number | null = null;
    const feedFor = new Market(id, start, currentTab.current === "trades", {
      stats: (next) => {
        if (previousLast !== null && next.last !== previousLast) {
          setLastDirection(next.last > previousLast ? 1 : -1);
        }
        previousLast = next.last;
        setStats(next);
      },
      book: setBooks,
      trades: setTrades,
      state: setFeed,
    });
    setMarket(feedFor);
    feedFor.start();
    return () => feedFor.dispose();
  }, [id, chartSpec]);

  useEffect(() => {
    currentTab.current = tab;
    store("chart.tab", tab);
    market?.setTrades(tab === "trades");
  }, [market, tab]);

  // Full motion while in front; when another app has focus the chart is only
  // glanced at, so redraw less often (it stops entirely once covered).
  useEffect(() => {
    const apply = () => setFrameRate(document.hasFocus() ? 60 : 15);
    apply();
    window.addEventListener("focus", apply);
    window.addEventListener("blur", apply);
    return () => {
      window.removeEventListener("focus", apply);
      window.removeEventListener("blur", apply);
    };
  }, []);

  // Covered or minimized for a minute: stop streaming until seen again.
  useEffect(() => {
    let timer: number | undefined;
    const onVisibility = () => {
      window.clearTimeout(timer);
      if (document.visibilityState === "hidden") {
        timer = window.setTimeout(() => market?.suspend(), 60_000);
      } else {
        market?.resume();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    // A new pair picked while hidden (the tray, a settings change) starts the clock too.
    if (document.visibilityState === "hidden") onVisibility();
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [market]);

  const pickBookStep = (next: number) => {
    setBookStep(next);
    store("chart.bookStep", String(next));
  };

  const setMode = (next: ChartMode) => {
    if (!interval) return;
    const updated = { ...modes, [interval.secs]: next };
    setModes(updated);
    store("chart.modeByInterval", JSON.stringify(updated));
  };

  return (
    <main className="flex h-screen flex-col select-none">
      <TitleBar className={cn(TITLE_INSET, "gap-2", !__WINDOWS__ && "pr-2")} maximizable>
        <InstrumentPicker
          watchlist={settings?.watchlist ?? []}
          instrument={instrument}
          id={id}
          onPick={(next) => void api.openChart(next)}
        />
        <span className="flex-1" />
        <LiveBadge state={feed} />
        <Button
          variant="ghost"
          size="icon-sm"
          title={chartSpec?.link?.label}
          aria-label={chartSpec?.link?.label}
          className="text-muted-foreground"
          disabled={!chartSpec?.link}
          onClick={() => void api.openLink(id)}
        >
          <ExternalLink />
        </Button>
      </TitleBar>

      <div className="flex min-h-0 flex-1">
        <section className="flex min-w-0 flex-1 flex-col gap-3 pb-4">
          <Hero
            stats={stats}
            span={chartSpec?.statsSpan ?? ""}
            decimals={decimals}
            lastDirection={lastDirection}
            insight={insight(visible, book)}
          />

          <div className="flex h-7 items-center gap-2 px-4">
            {interval && (
              <PillTabs
                label="K 线周期"
                value={interval.secs}
                options={(chartSpec?.intervals ?? []).map((i) => ({ value: i.secs, label: i.label }))}
                onChange={setIntervalSecs}
              />
            )}
            <span className="flex-1" />
            <ModeToggle mode={mode} onChange={setMode} />
          </div>

          {market && interval && chartSpec ? (
            <PriceChart
              market={market}
              interval={interval}
              mode={mode}
              colors={colors}
              paletteKey={chartKey}
              decimals={decimals}
              offline={feed === "offline"}
              source={chartSpec.source}
              dayOffset={chartSpec.dayOffset}
              onStats={setVisible}
            />
          ) : (
            <div className="panel mx-4 min-h-0 flex-1" />
          )}

          <StatsGrid stats={stats} span={chartSpec?.statsSpan ?? ""} decimals={decimals} spec={chartSpec} />
        </section>

        <aside className="flex w-70 shrink-0 flex-col border-l pt-2">
          <div className="px-3 pb-2.5">
            <PillTabs label="盘口与成交" value={tab} options={TABS} onChange={setTab} className="w-full" />
          </div>
          {tab === "book" ? (
            <OrderBook
              book={book}
              steps={bookSteps}
              step={step}
              onStep={pickBookStep}
              last={stats?.last ?? null}
              lastDirection={lastDirection}
              decimals={decimals}
              base={volumeUnit}
              quote={priceUnit}
            />
          ) : (
            <TradeList trades={trades} decimals={decimals} base={volumeUnit} quote={priceUnit} />
          )}
        </aside>
      </div>
    </main>
  );
}

/** One readable line about the stretch on screen, plus order book pressure while live. */
function insight(stats: VisibleStats | null, book: Book | null): string | null {
  if (!stats) return null;
  const parts = [`${stats.label} ${fmtPct(stats.changePct)}`, `振幅 ${stats.amplitudePct.toFixed(2)}%`];
  const share = stats.live ? bidShare(book) : null;
  if (share !== null) {
    parts.push(share >= 55 ? `买盘偏强 ${share.toFixed(0)}%` : share <= 45 ? `卖盘偏强 ${(100 - share).toFixed(0)}%` : "买卖均衡");
  }
  return parts.join(" · ");
}

function Hero(props: {
  stats: Stats | null;
  /** What the change covers, e.g. "24h". */
  span: string;
  decimals: number;
  lastDirection: 1 | -1 | 0;
  insight: string | null;
}) {
  const { stats, decimals } = props;
  const change = stats ? direction(stats.changePct) : 0;
  return (
    <div className="px-5 pt-2">
      <div className="flex items-baseline gap-3">
        {stats ? (
          <span
            // Remounted on every new price so the flash replays.
            key={stats.last}
            className="text-[34px] leading-none font-semibold tracking-tight tabular animate-[price-flash_1s_ease-out]"
            style={{ "--flash": props.lastDirection < 0 ? "var(--down)" : "var(--up)" } as CSSProperties}
          >
            {fmtPrice(stats.last, decimals)}
          </span>
        ) : (
          <span className="h-8.5 w-48 animate-pulse rounded-lg bg-fill-strong" />
        )}
        {stats && (
          <ChangeBadge
            pct={stats.changePct}
            amount={`${change > 0 ? "+" : change < 0 ? "−" : ""}${fmtPrice(Math.abs(stats.change), decimals)}`}
            span={props.span}
          />
        )}
      </div>
      <p className="mt-2 h-4 text-xs text-muted-foreground">{props.insight}</p>
    </div>
  );
}

function ModeToggle(props: { mode: ChartMode; onChange: (mode: ChartMode) => void }) {
  const options = [
    { value: "line", label: "折线", Icon: ChartLine },
    { value: "candle", label: "K 线", Icon: ChartCandlestick },
  ] as const;
  return (
    <div role="tablist" aria-label="图表类型" className="pill flex p-0.5">
      {options.map(({ value, label, Icon }) => {
        const selected = props.mode === value;
        return (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={selected}
            title={label}
            aria-label={label}
            onClick={() => props.onChange(value)}
            className={cn(
              "flex h-6 w-8 items-center justify-center rounded-full transition-all duration-200",
              selected
                ? "bg-white text-foreground shadow-[0_1px_2px_rgb(0_0_0/0.12)] dark:bg-white/16 dark:shadow-none"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon className="size-3.5" />
          </button>
        );
      })}
    </div>
  );
}

function LiveBadge(props: { state: FeedState }) {
  const live = props.state === "live";
  return (
    <span className="flex items-center gap-1.5 rounded-full px-2 text-xs text-muted-foreground">
      <span className="relative flex size-2">
        {live && <span className="absolute inset-0 animate-ping rounded-full bg-live opacity-60" />}
        <span
          className={cn(
            "relative size-2 rounded-full",
            live && "bg-live",
            props.state === "connecting" && "bg-busy",
            props.state === "offline" && "bg-destructive",
          )}
        />
      </span>
      {live ? "实时" : props.state === "connecting" ? "连接中" : "重连中"}
    </span>
  );
}

/** Instrument name that opens the watchlist, to switch the chart to another entry. */
function InstrumentPicker(props: {
  watchlist: Instrument[];
  instrument: Instrument | undefined;
  id: string;
  onPick: (id: string) => void;
}) {
  const label = props.instrument && instrumentLabel(props.instrument);
  const name = (
    <span className="text-sm">
      <span className="font-semibold">{label?.name}</span>
      {label?.detail && <span className="text-muted-foreground">{label.detail}</span>}
    </span>
  );
  const look = "-ml-1.5 flex h-6 items-center gap-1 rounded-full px-2 hover:bg-accent";
  if (props.watchlist.length < 2) {
    return <label className={cn("relative", look)}>{name}</label>;
  }
  const options = props.watchlist.map((i) => {
    const { name, detail } = instrumentLabel(i);
    return { value: instrumentId(i), label: `${name}${detail}` };
  });
  return (
    <Picker label="切换" value={props.id} options={options} onChange={props.onPick} className={look}>
      {name}
    </Picker>
  );
}

function StatsGrid(props: { stats: Stats | null; span: string; decimals: number; spec: ChartSpec | null }) {
  const { stats, span, decimals } = props;
  const items = [
    { label: `${span} 最高`, value: stats && fmtPrice(stats.high, decimals) },
    { label: `${span} 最低`, value: stats && fmtPrice(stats.low, decimals) },
    { label: `${span} 成交量`, value: stats && `${fmtCompact(stats.volume)} ${props.spec?.volumeUnit ?? ""}` },
    { label: `${span} 成交额`, value: stats && `${fmtCompact(stats.turnover)} ${props.spec?.turnoverUnit ?? ""}` },
  ];
  return (
    <dl className="grid grid-cols-4 gap-2 px-4">
      {items.map((item) => (
        <div key={item.label} className="min-w-0 rounded-xl bg-fill px-3 py-2">
          <dt className="text-2xs text-muted-foreground">{item.label}</dt>
          <dd className="mt-0.5 truncate font-medium tabular">{item.value ?? "—"}</dd>
        </div>
      ))}
    </dl>
  );
}
