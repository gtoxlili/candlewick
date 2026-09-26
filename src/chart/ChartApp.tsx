import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ChartCandlestick, ChartLine, ChevronsUpDown, ExternalLink } from "lucide-react";
import { Liveline, setFrameRate, setTrendColors, type CandlePoint, type LivelinePoint } from "liveline";
import { cn } from "cn";

import { PillTabs } from "@/components/PillTabs";
import { TRAFFIC_LIGHTS_INSET, TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import { api, subscribe, type Coin, type Settings } from "@/lib/api";
import { direction, fmtCompact, fmtPct, fmtPrice, priceDecimals } from "@/lib/format";
import {
  Market,
  WINDOWS,
  windowBySecs,
  type Book,
  type ChartData,
  type ChartMode,
  type FeedState,
  type Ticker,
  type TimeWindow,
  type Trade,
} from "./market";
import { bidShare, OrderBook } from "./OrderBook";
import { readChartColors, sameColors, type ChartColors } from "./palette";
import { TradeList } from "./TradeList";

type Tab = "book" | "trades";

const TABS = [
  { value: "book", label: "盘口" },
  { value: "trades", label: "成交" },
] as const satisfies readonly { value: Tab; label: string }[];

const WINDOW_TABS = WINDOWS.map((w) => ({ value: w.secs as number, label: w.label }));

/** Per-viewer view preferences; storage may be unavailable, which is fine. */
function load<T>(key: string, parse: (raw: string) => T | undefined, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return (raw !== null && parse(raw)) || fallback;
  } catch {
    return fallback;
  }
}

function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not persisted; the default comes back next time.
  }
}

const parseModes = (raw: string): Record<string, ChartMode> | undefined => {
  const value: unknown = JSON.parse(raw);
  return value && typeof value === "object" ? (value as Record<string, ChartMode>) : undefined;
};

const clockFull = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
const clockShort = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false });

export default function ChartApp() {
  const [symbol, setSymbol] = useState(
    () => new URLSearchParams(location.search).get("symbol") ?? "BTCUSDT",
  );
  const [settings, setSettings] = useState<Settings | null>(null);
  const [windowSecs, setWindowSecs] = useState(() =>
    load("chart.window", (raw) => WINDOWS.find((w) => String(w.secs) === raw)?.secs, 3600 as number),
  );
  const [modes, setModes] = useState(() => load("chart.modes", parseModes, {} as Record<string, ChartMode>));
  const [tab, setTab] = useState<Tab>(() =>
    load("chart.tab", (raw) => TABS.find((t) => t.value === raw)?.value, "book" as Tab),
  );
  const [chart, setChart] = useState<ChartData | null>(null);
  const [ticker, setTicker] = useState<Ticker | null>(null);
  const [lastDirection, setLastDirection] = useState<1 | -1 | 0>(0);
  const [book, setBook] = useState<Book | null>(null);
  const [trades, setTrades] = useState<Trade[]>([]);
  const [feed, setFeed] = useState<FeedState>("connecting");
  const [appearance, setAppearance] = useState(0);
  const [colors, setColors] = useState<ChartColors>(() => {
    const initial = readChartColors();
    setTrendColors(initial.up, initial.down);
    return initial;
  });
  const [chartKey, setChartKey] = useState(0);

  const market = useRef<Market | null>(null);
  const currentWindow = useRef<TimeWindow>(windowBySecs(windowSecs));
  const currentTab = useRef(tab);

  const win = windowBySecs(windowSecs);
  const mode: ChartMode = modes[win.secs] ?? win.mode;
  const coin: Coin | undefined = settings?.coins.find((c) => c.symbol === symbol);
  const base = coin?.base ?? symbol;
  const quote = coin?.quote ?? "";
  const decimals = priceDecimals(ticker?.last ?? 0, coin?.decimals ?? null);
  const scheme = settings?.colorScheme ?? "greenUp";
  const value = chart?.live?.close ?? chart?.candles.at(-1)?.close ?? ticker?.last ?? 0;
  const line = chart ? linePoints(chart, win.candle) : [];

  // Settings, the tray switching pairs, and showing the window.
  useEffect(() => {
    // The URL names the pair the window was opened for; a switch requested
    // while this page was still loading only reached the app, so read its
    // record once listening. A switch event is always newer than that read.
    let switched = false;
    const symbolEvents = api.onChartSymbol((next) => {
      switched = true;
      setSymbol(next);
    });
    const stops = [subscribe(api.onSettings(setSettings)), subscribe(symbolEvents)];
    symbolEvents
      .then(() => api.getChartSymbol())
      .then((stored) => {
        if (stored && !switched) setSymbol(stored);
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

  // Accent, appearance or the red/green convention changed: recolor the chart
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
    currentWindow.current = win;
    store("chart.window", String(win.secs));
    if (market.current) {
      setChart(null);
      market.current.setWindow(win);
    }
  }, [win]);

  // One feed per pair; window and tab changes reuse its socket.
  useEffect(() => {
    setChart(null);
    setTicker(null);
    setBook(null);
    setTrades([]);
    setLastDirection(0);
    let previousLast: number | null = null;
    const feedFor = new Market(symbol, currentWindow.current, currentTab.current === "trades", {
      chart: setChart,
      ticker: (next) => {
        if (previousLast !== null && next.last !== previousLast) {
          setLastDirection(next.last > previousLast ? 1 : -1);
        }
        previousLast = next.last;
        setTicker(next);
      },
      book: setBook,
      trades: setTrades,
      state: setFeed,
    });
    market.current = feedFor;
    feedFor.start();
    return () => {
      feedFor.dispose();
      market.current = null;
    };
  }, [symbol]);

  useEffect(() => {
    currentTab.current = tab;
    store("chart.tab", tab);
    market.current?.setTrades(tab === "trades");
  }, [tab]);

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
        timer = window.setTimeout(() => market.current?.suspend(), 60_000);
      } else {
        market.current?.resume();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  const setMode = (next: ChartMode) => {
    const updated = { ...modes, [win.secs]: next };
    setModes(updated);
    store("chart.modes", JSON.stringify(updated));
  };

  const formatTime = (t: number) => (win.secs <= 900 ? clockFull : clockShort).format(t * 1000);

  return (
    <main className="flex h-screen flex-col select-none">
      <TitleBar className={cn(TRAFFIC_LIGHTS_INSET, "gap-2 pr-2")}>
        <CoinPicker
          coins={settings?.coins ?? []}
          symbol={symbol}
          base={base}
          quote={quote}
          onPick={(next) => void api.openChart(next)}
        />
        <span className="flex-1" />
        <LiveBadge state={feed} />
        <Button
          variant="ghost"
          size="icon-sm"
          title="在币安打开"
          aria-label="在币安打开"
          className="text-muted-foreground"
          disabled={!coin}
          onClick={() => void api.openInBinance(symbol)}
        >
          <ExternalLink />
        </Button>
      </TitleBar>

      <div className="flex min-h-0 flex-1">
        <section className="flex min-w-0 flex-1 flex-col gap-3 pb-4">
          <Hero
            ticker={ticker}
            decimals={decimals}
            lastDirection={lastDirection}
            insight={insight(win, chart, book)}
          />

          <div className="flex items-center gap-2 px-4">
            <PillTabs label="时间范围" value={win.secs} options={WINDOW_TABS} onChange={setWindowSecs} />
            <span className="flex-1" />
            <ModeToggle mode={mode} onChange={setMode} />
          </div>

          <div className="relative mx-4 min-h-0 flex-1 overflow-hidden rounded-2xl border bg-white/45 dark:bg-white/3">
            <Liveline
              key={chartKey}
              mode="candle"
              data={line}
              value={value}
              candles={chart?.candles ?? []}
              candleWidth={win.candle}
              liveCandle={chart?.live ?? undefined}
              lineMode={mode === "line"}
              lineData={line}
              lineValue={value}
              window={win.secs}
              theme={colors.dark ? "dark" : "light"}
              color={colors.accent}
              loading={chart === null}
              formatValue={(v) => fmtPrice(v, decimals)}
              formatTime={formatTime}
              padding={{ top: 16, bottom: 28, left: 14 }}
            />
            {chart === null && feed === "offline" && (
              <p className="absolute inset-x-0 bottom-3 text-center text-xs text-muted-foreground">
                无法连接币安，正在重试…
              </p>
            )}
          </div>

          <Stats ticker={ticker} decimals={decimals} base={base} quote={quote} />
        </section>

        <aside className="flex w-70 shrink-0 flex-col border-l pt-2">
          <div className="px-3 pb-2.5">
            <PillTabs label="盘口与成交" value={tab} options={TABS} onChange={setTab} className="w-full" />
          </div>
          {tab === "book" ? (
            <OrderBook
              book={book}
              last={ticker?.last ?? null}
              lastDirection={lastDirection}
              decimals={decimals}
              base={base}
              quote={quote}
            />
          ) : (
            <TradeList trades={trades} decimals={decimals} base={base} quote={quote} />
          )}
        </aside>
      </div>
    </main>
  );
}

const withLive = (chart: ChartData): CandlePoint[] =>
  chart.live ? [...chart.candles, chart.live] : chart.candles;

/**
 * The line view: the first open, each candle's close at the end of its
 * bucket, then the live price now. Same density as the candles, so the
 * line/candle morph lines up and quiet markets don't draw as stairs.
 */
function linePoints(chart: ChartData, candle: number): LivelinePoint[] {
  const all = withLive(chart);
  if (all.length === 0) return [];
  const points: LivelinePoint[] = [{ time: all[0].time, value: all[0].open }];
  for (const c of chart.candles) points.push({ time: c.time + candle, value: c.close });
  const live = chart.live;
  if (live) points.push({ time: Math.min(Date.now() / 1000, live.time + candle), value: live.close });
  return points;
}

/** One readable line about the visible window and the order book. */
function insight(win: TimeWindow, chart: ChartData | null, book: Book | null): string | null {
  if (!chart) return null;
  const start = Date.now() / 1000 - win.secs;
  const visible = withLive(chart).filter((c) => c.time + win.candle > start);
  if (visible.length < 2) return null;
  const first = visible[0].open;
  const last = visible[visible.length - 1].close;
  let high = -Infinity;
  let low = Infinity;
  for (const c of visible) {
    high = Math.max(high, c.high);
    low = Math.min(low, c.low);
  }
  const parts = [`近${win.label} ${fmtPct(((last - first) / first) * 100)}`, `振幅 ${(((high - low) / low) * 100).toFixed(2)}%`];
  const share = bidShare(book);
  if (share !== null) {
    parts.push(share >= 55 ? `买盘偏强 ${share.toFixed(0)}%` : share <= 45 ? `卖盘偏强 ${(100 - share).toFixed(0)}%` : "买卖均衡");
  }
  return parts.join(" · ");
}

function Hero(props: { ticker: Ticker | null; decimals: number; lastDirection: 1 | -1 | 0; insight: string | null }) {
  const { ticker, decimals } = props;
  const change = ticker ? direction(ticker.changePct) : 0;
  return (
    <div className="px-5 pt-2">
      <div className="flex items-baseline gap-3">
        {ticker ? (
          <span
            // Remounted on every new price so the flash replays.
            key={ticker.last}
            className="text-[34px] leading-none font-semibold tracking-tight tabular animate-[price-flash_1s_ease-out]"
            style={{ "--flash": props.lastDirection < 0 ? "var(--down)" : "var(--up)" } as CSSProperties}
          >
            {fmtPrice(ticker.last, decimals)}
          </span>
        ) : (
          <span className="h-8.5 w-48 animate-pulse rounded-lg bg-black/5 dark:bg-white/8" />
        )}
        {ticker && (
          <span
            className={cn(
              "rounded-full px-2 py-0.5 text-xs font-medium tabular",
              change > 0 && "bg-up/12 text-up",
              change < 0 && "bg-down/12 text-down",
              change === 0 && "bg-black/5 text-muted-foreground dark:bg-white/8",
            )}
          >
            {fmtPct(ticker.changePct)} · {change > 0 ? "+" : change < 0 ? "−" : ""}
            {fmtPrice(Math.abs(ticker.change), decimals)}
            <span className="ml-1 opacity-70">24h</span>
          </span>
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
    <div role="tablist" aria-label="图表类型" className="flex rounded-full bg-black/5 p-0.5 dark:bg-white/8">
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
        {live && <span className="absolute inset-0 animate-ping rounded-full bg-(--green) opacity-60" />}
        <span
          className={cn(
            "relative size-2 rounded-full",
            live && "bg-(--green)",
            props.state === "connecting" && "bg-amber-400",
            props.state === "offline" && "bg-(--red)",
          )}
        />
      </span>
      {live ? "实时" : props.state === "connecting" ? "连接中" : "重连中"}
    </span>
  );
}

/** Pair name that opens the native pop-up menu of the watchlist. */
function CoinPicker(props: {
  coins: Coin[];
  symbol: string;
  base: string;
  quote: string;
  onPick: (symbol: string) => void;
}) {
  return (
    <label className="relative -ml-1.5 flex h-6 items-center gap-1 rounded-full px-2 hover:bg-accent">
      <span className="text-sm">
        <span className="font-semibold">{props.base}</span>
        {props.quote && <span className="text-muted-foreground">/{props.quote}</span>}
      </span>
      {props.coins.length > 1 && <ChevronsUpDown className="size-3 text-muted-foreground" />}
      {props.coins.length > 1 && (
        <select
          aria-label="切换交易对"
          className="absolute inset-0 appearance-none opacity-0"
          value={props.symbol}
          onChange={(e) => props.onPick(e.target.value)}
        >
          {props.coins.map((c) => (
            <option key={c.symbol} value={c.symbol}>
              {c.base}/{c.quote}
            </option>
          ))}
        </select>
      )}
    </label>
  );
}

function Stats(props: { ticker: Ticker | null; decimals: number; base: string; quote: string }) {
  const { ticker, decimals } = props;
  const items = [
    { label: "24h 最高", value: ticker && fmtPrice(ticker.high, decimals) },
    { label: "24h 最低", value: ticker && fmtPrice(ticker.low, decimals) },
    { label: "24h 成交量", value: ticker && `${fmtCompact(ticker.volume)} ${props.base}` },
    { label: "24h 成交额", value: ticker && `${fmtCompact(ticker.quoteVolume)} ${props.quote}` },
  ];
  return (
    <dl className="grid grid-cols-4 gap-2 px-4">
      {items.map((item) => (
        <div key={item.label} className="min-w-0 rounded-xl bg-black/[0.035] px-3 py-2 dark:bg-white/5">
          <dt className="text-[11px] text-muted-foreground">{item.label}</dt>
          <dd className="mt-0.5 truncate font-medium tabular">{item.value ?? "—"}</dd>
        </div>
      ))}
    </dl>
  );
}
