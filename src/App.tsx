import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { ChevronDown, ChevronUp, LoaderCircle, Search, X } from "lucide-react";
import { cn } from "cn";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  api,
  loadPairs,
  MAX_COINS,
  parsePair,
  searchPairs,
  type Coin,
  type Pair,
  type Settings,
  type Status,
} from "@/lib/api";

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [loginItem, setLoginItem] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const saveSeq = useRef(0);
  /** Last settings the app accepted, to roll back a rejected change. */
  const saved = useRef<Settings | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // A status event is always newer than the initial fetch below.
    let gotEvent = false;
    api
      .onStatus((next) => {
        gotEvent = true;
        setStatus(next);
      })
      .then((fn) => (disposed ? fn() : (unlisten = fn)));
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
    return () => {
      disposed = true;
      unlisten?.();
    };
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

  if (!settings) {
    return (
      <main className="flex h-screen items-center justify-center p-6 text-sm text-muted-foreground">
        {error ?? <LoaderCircle className="size-5 animate-spin" />}
      </main>
    );
  }

  const coins = settings.coins;
  const setCoins = (next: Coin[]) => update({ ...settings, coins: next });
  const move = (index: number, delta: number) => {
    const next = [...coins];
    const [coin] = next.splice(index, 1);
    next.splice(index + delta, 0, coin);
    void setCoins(next);
  };

  return (
    <main className="flex h-screen flex-col select-none">
      <div className="flex-1 space-y-5 overflow-y-auto px-5 pt-4 pb-6">
        <Section title="币种" footnote="打开「菜单栏」的币种会直接显示在菜单栏上；点下拉菜单中的币种可打开币安交易页。">
          <CoinSearch
            existing={coins}
            full={coins.length >= MAX_COINS}
            onAdd={(coin) => void setCoins([...coins, { ...coin, pinned: coins.length === 0 }])}
          />
          <Group>
            {coins.length === 0 ? (
              <p className="px-3.5 py-6 text-center text-sm text-muted-foreground">
                还没有币种，在上方搜索并添加
              </p>
            ) : (
              coins.map((coin, index) => (
                <div key={coin.symbol} className="flex h-11 items-center gap-1 pr-2 pl-3.5">
                  <span className="min-w-0 flex-1 truncate text-sm">
                    <span className="font-medium">{coin.base}</span>
                    <span className="text-muted-foreground">/{coin.quote}</span>
                  </span>
                  <Label className="mr-2 gap-2 text-xs font-normal text-muted-foreground">
                    菜单栏
                    <Switch
                      size="sm"
                      checked={coin.pinned}
                      onCheckedChange={(pinned) =>
                        void setCoins(coins.map((c) => (c.symbol === coin.symbol ? { ...c, pinned } : c)))
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
                    onClick={() => void setCoins(coins.filter((c) => c.symbol !== coin.symbol))}
                  >
                    <X />
                  </Button>
                </div>
              ))
            )}
          </Group>
        </Section>

        <Section title="显示">
          <Group>
            <SwitchRow
              label="菜单栏显示币种名称"
              checked={settings.showSymbol}
              onChange={(showSymbol) => void update({ ...settings, showSymbol })}
            />
            <SwitchRow
              label="菜单栏显示 24 小时涨跌幅"
              checked={settings.showChange}
              onChange={(showChange) => void update({ ...settings, showChange })}
            />
            <SwitchRow
              label="红涨绿跌"
              hint="下拉菜单中涨跌幅的配色"
              checked={settings.colorScheme === "redUp"}
              onChange={(redUp) => void update({ ...settings, colorScheme: redUp ? "redUp" : "greenUp" })}
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
          <p className="rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</p>
        )}
      </div>

      <footer className="flex items-center gap-2 border-t bg-card/60 px-5 py-2 text-xs text-muted-foreground">
        <span
          className={cn(
            "size-2 shrink-0 rounded-full",
            status?.tone === "live" && "bg-emerald-500",
            status?.tone === "busy" && "bg-amber-400",
            status?.tone === "error" && "bg-red-500",
            (!status || status.tone === "idle") && "bg-muted-foreground/40",
          )}
        />
        <span className="truncate">{status?.label ?? "—"}</span>
        <span className="ml-auto shrink-0">数据来自币安现货</span>
      </footer>
    </main>
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
  return <div className="divide-y rounded-xl border bg-card shadow-xs">{children}</div>;
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
    <div className="flex min-h-11 items-center justify-between gap-4 px-3.5 py-2">
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

function CoinSearch(props: { existing: Coin[]; full: boolean; onAdd: (coin: Coin) => void }) {
  const [query, setQuery] = useState("");
  const [pairs, setPairs] = useState<Pair[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  // -1 = follow the first pair that can still be added.
  const [active, setActive] = useState(-1);

  const ensurePairs = () => {
    if (pairs || loading) return;
    setLoading(true);
    loadPairs()
      .then((list) => {
        setPairs(list);
        setFailed(false);
      })
      .catch(() => setFailed(true))
      .finally(() => setLoading(false));
  };

  const taken = new Set(props.existing.map((c) => c.symbol));
  const manual = !pairs && failed ? parsePair(query) : null;
  const options: Coin[] = manual
    ? [manual]
    : (pairs ? searchPairs(pairs, query) : []).map((p) => ({ ...p, pinned: false }));

  const current =
    active >= 0 && active < options.length ? active : options.findIndex((o) => !taken.has(o.symbol));

  const add = (coin: Coin | undefined) => {
    if (!coin || taken.has(coin.symbol) || props.full) return;
    props.onAdd(coin);
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
      <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        value={query}
        disabled={props.full}
        placeholder={props.full ? `最多 ${MAX_COINS} 个币种` : "添加币种：搜索交易对，如 SOL、ETHBTC"}
        className="h-9 rounded-xl bg-card pl-9 text-sm shadow-xs dark:bg-card"
        spellCheck={false}
        autoCorrect="off"
        onFocus={ensurePairs}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(-1);
          ensurePairs();
        }}
        onKeyDown={onKeyDown}
      />
      {query.trim() !== "" && (
        <div className="absolute inset-x-0 top-full z-10 mt-1 overflow-hidden rounded-xl border bg-popover p-1 text-popover-foreground shadow-lg">
          {loading && !pairs ? (
            <p className="flex items-center gap-2 px-2.5 py-1.5 text-sm text-muted-foreground">
              <LoaderCircle className="size-4 animate-spin" /> 正在获取交易对…
            </p>
          ) : options.length === 0 ? (
            <p className="px-2.5 py-1.5 text-sm text-muted-foreground">
              {failed ? "无法获取交易对列表，请输入完整交易对，如 SOLUSDT" : "没有匹配的交易对"}
            </p>
          ) : (
            options.map((coin, index) => {
              const added = taken.has(coin.symbol);
              return (
                <button
                  key={coin.symbol}
                  type="button"
                  disabled={added}
                  onMouseEnter={() => setActive(index)}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => add(coin)}
                  className={cn(
                    "flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left text-sm",
                    index === current && !added && "bg-accent text-accent-foreground",
                    added && "text-muted-foreground",
                  )}
                >
                  <span>
                    <span className="font-medium">{coin.base}</span>
                    <span className="text-muted-foreground">/{coin.quote}</span>
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {added ? "已添加" : manual ? "手动添加" : ""}
                  </span>
                </button>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}
