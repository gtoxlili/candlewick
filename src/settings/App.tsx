import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { ChevronDown, ChevronUp, LoaderCircle, Search, X } from "lucide-react";
import { cn } from "cn";

import { TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  api,
  instrumentId,
  MAX_INSTRUMENTS,
  subscribe,
  type Candidate,
  type Instrument,
  type Search as SearchResults,
  type Settings,
  type Status,
} from "@/lib/api";

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [loginItem, setLoginItem] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scrolled, setScrolled] = useState(false);
  const saveSeq = useRef(0);
  /** Last settings the app accepted, to roll back a rejected change. */
  const saved = useRef<Settings | null>(null);

  useEffect(() => {
    // A status event is always newer than the initial fetch below.
    let gotEvent = false;
    const stop = subscribe(
      api.onStatus((next) => {
        gotEvent = true;
        setStatus(next);
      }),
    );
    Promise.all([api.getSettings(), api.getStatus(), api.getLoginItem()])
      .then(([s, st, login]) => {
        saved.current = s;
        setSettings(s);
        if (!gotEvent) setStatus(st);
        setLoginItem(login);
      })
      .catch((e: unknown) => setError(String(e)))
      // Show the (initially hidden) window now that there is content. Not via
      // requestAnimationFrame: WebKit doesn't render a hidden window, so that
      // callback would never fire.
      .finally(() => void api.ready());
    return stop;
  }, []);

  // Apply immediately, then adopt the app's normalized copy (or roll back if
  // it was rejected) unless a newer change was made in the meantime.
  const update = async (next: Settings) => {
    const seq = ++saveSeq.current;
    setSettings(next);
    try {
      saved.current = await api.saveSettings(next);
      if (seq === saveSeq.current) setSettings(saved.current);
      setError(null);
    } catch (e) {
      if (seq === saveSeq.current) setSettings(saved.current);
      setError(String(e));
    }
  };

  const toggleLoginItem = async (enabled: boolean) => {
    setLoginItem(enabled);
    try {
      setLoginItem(await api.setLoginItem(enabled));
      setError(null);
    } catch (e) {
      setLoginItem(!enabled);
      setError(String(e));
    }
  };

  return (
    <main className="flex h-screen flex-col select-none">
      <TitleBar className="justify-center" divider={scrolled}>
        <h1 className="text-sm font-semibold">设置</h1>
      </TitleBar>

      {!settings ? (
        <div className="flex flex-1 items-center justify-center p-6 text-muted-foreground">
          {error ?? <LoaderCircle className="size-5 animate-spin" />}
        </div>
      ) : (
        <div
          className="flex-1 space-y-5 overflow-y-auto px-5 pt-3 pb-6"
          onScroll={(e) => setScrolled(e.currentTarget.scrollTop > 0)}
        >
          <WatchlistSection
            watchlist={settings.watchlist}
            onChange={(watchlist) => void update({ ...settings, watchlist })}
          />

          <Section title="显示">
            <Group>
              <SwitchRow
                label="菜单栏显示币种名称"
                checked={settings.showSymbol}
                onChange={(showSymbol) => void update({ ...settings, showSymbol })}
              />
              <SwitchRow
                label="菜单栏显示 24 小时涨跌幅"
                hint="价格与涨跌幅分上下两排显示"
                checked={settings.showChange}
                onChange={(showChange) => void update({ ...settings, showChange })}
              />
              <SwitchRow
                label="红涨绿跌"
                hint="下拉菜单与 K 线图的涨跌配色"
                checked={settings.colorScheme === "redUp"}
                onChange={(redUp) =>
                  void update({ ...settings, colorScheme: redUp ? "redUp" : "greenUp" })
                }
              />
            </Group>
          </Section>

          <Section title="通用">
            <Group>
              <SwitchRow
                label="登录时启动"
                hint="开机后自动出现在菜单栏"
                checked={loginItem ?? false}
                disabled={loginItem === null}
                onChange={(enabled) => void toggleLoginItem(enabled)}
              />
            </Group>
          </Section>

          {error && (
            <p className="rounded-lg bg-destructive/10 px-3 py-2 text-destructive">{error}</p>
          )}
        </div>
      )}

      <footer className="flex h-8 shrink-0 items-center gap-2 border-t px-5 text-xs text-muted-foreground">
        <span
          className={cn(
            "size-2 shrink-0 rounded-full",
            status?.tone === "live" && "bg-(--green)",
            status?.tone === "busy" && "bg-amber-400",
            status?.tone === "error" && "bg-(--red)",
            (!status || status.tone === "idle") && "bg-muted-foreground/40",
          )}
        />
        <span className="truncate">{status?.label ?? "—"}</span>
        <span className="ml-auto shrink-0">数据来自币安现货</span>
      </footer>
    </main>
  );
}

function WatchlistSection(props: { watchlist: Instrument[]; onChange: (watchlist: Instrument[]) => void }) {
  const { watchlist: coins, onChange } = props;
  const move = (index: number, delta: number) => {
    const next = [...coins];
    const [coin] = next.splice(index, 1);
    next.splice(index + delta, 0, coin);
    onChange(next);
  };
  return (
    <Section
      title="币种"
      footnote="菜单栏只显示一个币种，打开哪个的「菜单栏」就显示哪个；点下拉菜单里的币种可查看 K 线与盘口。"
    >
      <InstrumentSearch
        existing={coins}
        full={coins.length >= MAX_INSTRUMENTS}
        onAdd={(coin) => onChange([...coins, { ...coin, pinned: coins.length === 0 }])}
      />
      <Group>
        {coins.length === 0 ? (
          <p className="px-3.5 py-6 text-center text-muted-foreground">还没有币种，在上方搜索并添加</p>
        ) : (
          coins.map((coin, index) => (
            <div key={instrumentId(coin)} className="flex h-10 items-center gap-0.5 pr-1.5 pl-3.5">
              <span className="min-w-0 flex-1 truncate">
                <span className="font-medium">{coin.base}</span>
                <span className="text-muted-foreground">/{coin.quote}</span>
              </span>
              <Label className="mr-2 gap-2 text-xs font-normal text-muted-foreground">
                菜单栏
                <Switch
                  size="sm"
                  checked={coin.pinned}
                  onCheckedChange={(pinned) =>
                    // One pair in the menu bar: switching one on switches the others off.
                    onChange(
                      coins.map((c) =>
                        c === coin ? { ...c, pinned } : pinned ? { ...c, pinned: false } : c,
                      ),
                    )
                  }
                />
              </Label>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="上移"
                className="text-muted-foreground"
                disabled={index === 0}
                onClick={() => move(index, -1)}
              >
                <ChevronUp />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="下移"
                className="text-muted-foreground"
                disabled={index === coins.length - 1}
                onClick={() => move(index, 1)}
              >
                <ChevronDown />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={`删除 ${coin.base}/${coin.quote}`}
                className="text-muted-foreground hover:text-destructive"
                onClick={() => onChange(coins.filter((c) => c !== coin))}
              >
                <X />
              </Button>
            </div>
          ))
        )}
      </Group>
    </Section>
  );
}

/** macOS System Settings style: small title above a grouped box, optional footnote below. */
function Section(props: { title: string; footnote?: string; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h2 className="px-1 text-xs font-medium text-muted-foreground">{props.title}</h2>
      {props.children}
      {props.footnote && <p className="px-1 text-xs text-muted-foreground">{props.footnote}</p>}
    </section>
  );
}

function Group({ children }: { children: ReactNode }) {
  return <div className="divide-y rounded-xl border bg-card">{children}</div>;
}

function SwitchRow(props: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="flex min-h-10 items-center justify-between gap-4 px-3.5 py-2">
      <div className="space-y-0.5">
        <Label htmlFor={id} className="font-normal">
          {props.label}
        </Label>
        {props.hint && <p className="text-xs text-muted-foreground">{props.hint}</p>}
      </div>
      <Switch
        id={id}
        checked={props.checked}
        disabled={props.disabled}
        onCheckedChange={props.onChange}
      />
    </div>
  );
}

function InstrumentSearch(props: { existing: Instrument[]; full: boolean; onAdd: (instrument: Instrument) => void }) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResults | null>(null);
  const [loading, setLoading] = useState(false);
  // -1 = follow the first candidate that can still be added.
  const [active, setActive] = useState(-1);
  /** Only the latest search may show its results. */
  const latest = useRef(0);

  const run = (text: string) => {
    const seq = ++latest.current;
    setLoading(true);
    api
      .search(text)
      .then((found) => seq === latest.current && setResults(found))
      .catch(() => seq === latest.current && setResults({ candidates: [], degraded: true }))
      .finally(() => seq === latest.current && setLoading(false));
  };

  const taken = new Set(props.existing.map(instrumentId));
  const options: Candidate[] = query.trim() ? (results?.candidates ?? []) : [];

  const current =
    active >= 0 && active < options.length ? active : options.findIndex((o) => !taken.has(instrumentId(o)));

  const add = (candidate: Candidate | undefined) => {
    if (!candidate || taken.has(instrumentId(candidate)) || props.full) return;
    const { manual: _, ...instrument } = candidate;
    props.onAdd(instrument);
    setQuery("");
    setActive(-1);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive(Math.min(current + 1, options.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive(Math.max(current - 1, 0));
    } else if (event.key === "Enter") {
      event.preventDefault();
      add(options[current]);
    } else if (event.key === "Escape") {
      setQuery("");
    }
  };

  return (
    <div className="relative">
      <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
      <Input
        value={query}
        disabled={props.full}
        placeholder={props.full ? `最多 ${MAX_INSTRUMENTS} 个币种` : "添加币种：搜索交易对，如 SOL、ETHBTC"}
        className="h-8 rounded-lg bg-card pl-8 dark:bg-card"
        spellCheck={false}
        autoCorrect="off"
        // Starts loading the pair list before the first keystroke.
        onFocus={() => results || loading || run("")}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(-1);
          run(e.target.value);
        }}
        onKeyDown={onKeyDown}
      />
      {query.trim() !== "" && (
        <div className="absolute inset-x-0 top-full z-10 mt-1 overflow-hidden rounded-xl border bg-popover p-1 text-popover-foreground shadow-lg backdrop-blur-xl">
          {options.length === 0 ? (
            loading ? (
              <p className="flex items-center gap-2 px-2.5 py-1.5 text-muted-foreground">
                <LoaderCircle className="size-4 animate-spin" /> 正在获取交易对…
              </p>
            ) : (
              <p className="px-2.5 py-1.5 text-muted-foreground">
                {results?.degraded ? "无法获取交易对列表，请输入完整交易对，如 SOLUSDT" : "没有匹配的交易对"}
              </p>
            )
          ) : (
            options.map((candidate, index) => {
              const added = taken.has(instrumentId(candidate));
              return (
                <button
                  key={instrumentId(candidate)}
                  type="button"
                  disabled={added}
                  onMouseEnter={() => setActive(index)}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => add(candidate)}
                  className={cn(
                    "flex w-full items-center justify-between rounded-md px-2.5 py-1 text-left",
                    index === current && !added && "bg-primary text-primary-foreground",
                    added && "text-muted-foreground",
                  )}
                >
                  <span>
                    <span className="font-medium">{candidate.base}</span>
                    <span className={index === current && !added ? "opacity-80" : "text-muted-foreground"}>
                      /{candidate.quote}
                    </span>
                  </span>
                  <span className="text-xs opacity-70">{added ? "已添加" : candidate.manual ? "手动添加" : ""}</span>
                </button>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}
