import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ChartCandlestick, ChartLine, ChevronsUpDown, ExternalLink } from "lucide-react";
import { setFrameRate, setTrendColors } from "liveline";
import { cn } from "cn";

import { PillTabs } from "@/components/PillTabs";
import { TRAFFIC_LIGHTS_INSET, TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import { api, subscribe, type Coin, type Settings } from "@/lib/api";
import { direction, fmtCompact, fmtPct, fmtPrice, priceDecimals } from "@/lib/format";
import {
  INTERVALS,
  intervalBySecs,
  Market,
  type Book,
  type ChartMode,
  type FeedState,
  type Ticker,
  type Trade,
} from "./market";
import { bidShare, OrderBook } from "./OrderBook";
import { readChartColors, sameColors, type ChartColors } from "./palette";
import { load, store } from "./prefs";
import { PriceChart, type VisibleStats } from "./PriceChart";
import { TradeList } from "./TradeList";

type Tab = "book" | "trades";

const TABS = [
  { value: "book", label: "盘口" },
  { value: "trades", label: "成交" },
] as const satisfies readonly { value: Tab; label: string }[];

const INTERVAL_TABS = INTERVALS.map((i) => ({ value: i.secs as number, label: i.label }));

const parseModes = (raw: string): Record<string, ChartMode> | undefined => {
  const value: unknown = JSON.parse(raw);
  return value && typeof value === "object" ? (value as Record<string, ChartMode>) : undefined;
};

export default function ChartApp() {
  const [symbol, setSymbol] = useState(
    () => new URLSearchParams(location.search).get("symbol") ?? "BTCUSDT",
  );
  const [settings, setSettings] = useState<Settings | null>(null);
  const [intervalSecs, setIntervalSecs] = useState(() =>
    load("chart.interval", (raw) => INTERVALS.find((i) => String(i.secs) === raw)?.secs, 60 as number),
  );
  const [modes, setModes] = useState(() =>
    load("chart.modeByInterval", parseModes, {} as Record<string, ChartMode>),
  );
  const [tab, setTab] = useState<Tab>(() =>
    load("chart.tab", (raw) => TABS.find((t) => t.value === raw)?.value, "book" as Tab),
  );
  const [market, setMarket] = useState<Market | null>(null);
  const [stats, setStats] = useState<VisibleStats | null>(null);
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

  const currentInterval = useRef(intervalSecs);
  const currentTab = useRef(tab);

  const interval = intervalBySecs(intervalSecs);
  const mode: ChartMode = modes[interval.secs] ?? interval.mode;
  const coin: Coin | undefined = settings?.coins.find((c) => c.symbol === symbol);
  const base = coin?.base ?? symbol;
  const quote = coin?.quote ?? "";
  const decimals = priceDecimals(ticker?.last ?? 0, coin?.decimals ?? null);
  const scheme = settings?.colorScheme ?? "greenUp";

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
    currentInterval.current = intervalSecs;
    store("chart.interval", String(intervalSecs));
  }, [intervalSecs]);

  // One feed per pair; interval and tab changes reuse its socket.
  useEffect(() => {
    setTicker(null);
    setBook(null);
    setTrades([]);
    setLastDirection(0);
    let previousLast: number | null = null;
    const feedFor = new Market(symbol, intervalBySecs(currentInterval.current), currentTab.current === "trades", {
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
    setMarket(feedFor);
    feedFor.start();
    return () => feedFor.dispose();
  }, [symbol]);

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

  const setMode = (next: ChartMode) => {
    const updated = { ...modes, [interval.secs]: next };
    setModes(updated);
    store("chart.modeByInterval", JSON.stringify(updated));
  };

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
            insight={insight(stats, book)}
          />

          <div className="flex items-center gap-2 px-4">
            <PillTabs label="K 线周期" value={interval.secs} options={INTERVAL_TABS} onChange={setIntervalSecs} />
            <span className="flex-1" />
            <ModeToggle mode={mode} onChange={setMode} />
          </div>

          {market ? (
            <PriceChart
              market={market}
              interval={interval}
              mode={mode}
              colors={colors}
              paletteKey={chartKey}
              decimals={decimals}
              offline={feed === "offline"}
              onStats={setStats}
            />
          ) : (
            <div className="mx-4 min-h-0 flex-1 rounded-2xl border bg-white/45 dark:bg-white/3" />
          )}

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
