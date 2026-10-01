import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { ChevronDown, ChevronRight, ChevronUp, LoaderCircle, Search, X } from "lucide-react";
import { cn } from "cn";

import { Picker } from "@/components/Picker";
import { TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { BAR } from "@/lib/platform";
import {
  api,
  EXCHANGES,
  instrumentId,
  instrumentLabel,
  marketLabel,
  MAX_INSTRUMENTS,
  subscribe,
  type ApiKey,
  type Candidate,
  type Exchange,
  type ExchangeKey,
  type Instrument,
  type Longbridge,
  type LongbridgeKeys,
  type Search as SearchResults,
  type Settings,
  type Update,
  type UpdateState,
} from "@/lib/api";

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [loginItem, setLoginItem] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scrolled, setScrolled] = useState(false);
  const saveSeq = useRef(0);
  /** Last settings the app accepted, to roll back a rejected change. */
  const saved = useRef<Settings | null>(null);

  useEffect(() => {
    Promise.all([api.getSettings(), api.getLoginItem()])
      .then(([s, login]) => {
        saved.current = s;
        setSettings(s);
        setLoginItem(login);
      })
      .catch((e: unknown) => setError(String(e)))
      // Show the (initially hidden) window now that there is content. Not via
      // requestAnimationFrame: WebKit doesn't render a hidden window, so that
      // callback would never fire.
      .finally(() => void api.ready());
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
      <TitleBar title="设置" divider={scrolled} />

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
            exchange={settings.exchange}
            watchlist={settings.watchlist}
            onChange={(watchlist) => void update({ ...settings, watchlist })}
          />

          <ExchangeSection
            exchange={settings.exchange}
            onChange={(exchange) => void update({ ...settings, exchange })}
          />

          <ApiKeysSection />

          <LongbridgeSection />

          <Section title="显示">
            <Group>
              <SwitchRow
                label={`${BAR}显示名称`}
                checked={settings.showSymbol}
                onChange={(showSymbol) => void update({ ...settings, showSymbol })}
              />
              <SwitchRow
                label={`${BAR}显示涨跌幅`}
                hint="价格与涨跌幅分上下两排显示"
                checked={settings.showChange}
                onChange={(showChange) => void update({ ...settings, showChange })}
              />
              <SwitchRow
                label="红涨绿跌"
                hint={__WINDOWS__ ? "任务栏、菜单与 K 线图的涨跌配色" : "下拉菜单与 K 线图的涨跌配色"}
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
                label={__WINDOWS__ ? "开机时启动" : "登录时启动"}
                hint={`开机后自动出现在${BAR}`}
                checked={loginItem ?? false}
                disabled={loginItem === null}
                onChange={(enabled) => void toggleLoginItem(enabled)}
              />
              <SwitchRow
                label="自动更新"
                hint="在后台下载新版本，锁屏或显示器关闭时重新启动"
                checked={settings.autoUpdate}
                onChange={(autoUpdate) => void update({ ...settings, autoUpdate })}
              />
            </Group>
          </Section>

          {error && (
            <p className="rounded-lg bg-destructive/10 px-3 py-2 text-destructive">{error}</p>
          )}
        </div>
      )}

      <AppFooter />
    </main>
  );
}

const WATCHLIST_FOOTNOTE = __WINDOWS__
  ? "任务栏只显示一个，打开哪个的「任务栏」就显示哪个；点任务栏上的行情或托盘图标，在菜单里选名称可查看 K 线与盘口。"
  : "菜单栏只显示一个，打开哪个的「菜单栏」就显示哪个；点下拉菜单里的名称可查看 K 线与盘口。";

function WatchlistSection(props: {
  exchange: Exchange;
  watchlist: Instrument[];
  onChange: (watchlist: Instrument[]) => void;
}) {
  const { watchlist, onChange } = props;
  const move = (index: number, delta: number) => {
    const next = [...watchlist];
    const [moved] = next.splice(index, 1);
    next.splice(index + delta, 0, moved);
    onChange(next);
  };
  return (
    <Section title="自选" footnote={WATCHLIST_FOOTNOTE}>
      <InstrumentSearch
        // Another exchange clears the search: its results were the old exchange's pairs.
        key={props.exchange}
        existing={watchlist}
        full={watchlist.length >= MAX_INSTRUMENTS}
        onAdd={(added) => onChange([...watchlist, { ...added, pinned: watchlist.length === 0 }])}
      />
      <Group>
        {watchlist.length === 0 ? (
          <p className="px-3.5 py-6 text-center text-muted-foreground">还没有自选，在上方搜索并添加</p>
        ) : (
          watchlist.map((item, index) => (
            <div key={instrumentId(item)} className="flex h-10 items-center gap-0.5 pr-1.5 pl-3.5">
              <InstrumentName instrument={item} className="min-w-0 flex-1 truncate" />
              <Label className="mr-2 gap-2 text-xs font-normal text-muted-foreground">
                {BAR}
                <Switch
                  size="sm"
                  checked={item.pinned}
                  onCheckedChange={(pinned) =>
                    // One pair in the bar: switching one on switches the others off.
                    onChange(
                      watchlist.map((c) =>
                        c === item ? { ...c, pinned } : pinned ? { ...c, pinned: false } : c,
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
                disabled={index === watchlist.length - 1}
                onClick={() => move(index, 1)}
              >
                <ChevronDown />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={`删除 ${instrumentLabel(item).name}`}
                className="text-muted-foreground hover:text-destructive"
                onClick={() => onChange(watchlist.filter((c) => c !== item))}
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

/** Which exchange the pairs come from. */
function ExchangeSection(props: { exchange: Exchange; onChange: (exchange: Exchange) => void }) {
  const current = EXCHANGES.find((e) => e.value === props.exchange);
  return (
    <Section title="加密货币" footnote="公开行情，无需账号。换一家交易所，会从自选中移除原交易所的币对。">
      <Group>
        <div className="flex min-h-10 items-center justify-between gap-4 px-3.5 py-2">
          <span>交易所</span>
          <Picker
            label="交易所"
            value={props.exchange}
            options={EXCHANGES}
            onChange={(value) => {
              const picked = EXCHANGES.find((e) => e.value === value);
              if (picked) props.onChange(picked.value);
            }}
            className="-mr-1.5 flex h-7 items-center gap-1 rounded-md px-2 hover:bg-accent"
          >
            <span>{current?.label}</span>
          </Picker>
        </div>
      </Group>
    </Section>
  );
}

/** Where each exchange lets one create an API key. */
const API_PAGES: Record<Exchange, string> = {
  binance: "https://www.binance.com/zh-CN/my/settings/api-management",
  bybit: "https://www.bybit.com/app/user/api-management",
  okx: "https://www.okx.com/zh-hans/account/my-api",
};

const EMPTY_KEY: ApiKey = { key: "", secret: "", passphrase: "" };

/** What each exchange's API page calls the parts of a key; OKX adds a passphrase. */
function keyFields(exchange: Exchange): { field: keyof ApiKey; label: string; secret: boolean }[] {
  const okx = exchange === "okx";
  return [
    { field: "key", label: "API Key", secret: false },
    { field: "secret", label: okx ? "Secret Key" : "API Secret", secret: true },
    ...(okx ? [{ field: "passphrase" as const, label: "Passphrase", secret: true }] : []),
  ];
}

/** Read-only API keys, for holdings: one per exchange, checked when saved. */
function ApiKeysSection() {
  const [keys, setKeys] = useState<ExchangeKey[] | null>(null);
  const [editing, setEditing] = useState<Exchange | null>(null);
  const [form, setForm] = useState<ApiKey>(EMPTY_KEY);
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<{ exchange: Exchange; message: string } | null>(null);

  useEffect(() => {
    api.getExchangeKeys().then(setKeys).catch(() => setKeys([]));
  }, []);

  const edit = (exchange: Exchange | null) => {
    setEditing(exchange);
    setForm(EMPTY_KEY);
    setProblem(null);
  };

  const save = (exchange: Exchange, key: ApiKey | null) => {
    setSaving(true);
    setProblem(null);
    api
      .setExchangeKey(exchange, key)
      .then((saved) => {
        setKeys(saved);
        edit(null);
      })
      .catch((e: unknown) => setProblem({ exchange, message: String(e) }))
      .finally(() => setSaving(false));
  };

  if (!keys) return null;
  const configured = keys.some((k) => k.key !== null);

  return (
    <Section
      title="持仓"
      footnote="填写只读 API Key 后，菜单里会显示总资产，持仓窗口列出现货、资金、理财和合约。创建时只勾选读取权限，能交易或提币的 Key 不会被接受。Key 只保存在这台电脑上。"
    >
      <Group>
        {keys.map(({ exchange, key }) => {
          const name = EXCHANGES.find((e) => e.value === exchange)?.label ?? exchange;
          const open = editing === exchange;
          const fields = keyFields(exchange);
          const complete = fields.every(({ field }) => (form[field] ?? "").trim() !== "");
          return (
            <div key={exchange}>
              <div className="flex min-h-10 items-center gap-2 px-3.5 py-2">
                <span className="w-12 shrink-0">{name}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground">
                  {key ?? (open ? "" : "未填写")}
                </span>
                {!open && (
                  <Button variant="ghost" size="sm" disabled={saving} onClick={() => edit(exchange)}>
                    {key ? "更换" : "添加"}
                  </Button>
                )}
                {key && !open && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="hover:text-destructive"
                    disabled={saving}
                    onClick={() => save(exchange, null)}
                  >
                    移除
                  </Button>
                )}
              </div>
              {open && (
                <div className="space-y-2 px-3.5 pb-3">
                  {fields.map(({ field, label, secret }) => (
                    <label key={field} className="flex items-center gap-3">
                      <span className="w-24 shrink-0 text-muted-foreground">{label}</span>
                      <Input
                        type={secret ? "password" : "text"}
                        value={form[field] ?? ""}
                        className="h-7 rounded-md font-mono text-xs"
                        spellCheck={false}
                        autoCorrect="off"
                        disabled={saving}
                        onChange={(e) => setForm({ ...form, [field]: e.target.value })}
                      />
                    </label>
                  ))}
                  <div className="flex items-center justify-end gap-2 pt-1">
                    <a
                      href={API_PAGES[exchange]}
                      target="_blank"
                      rel="noreferrer"
                      className="mr-auto text-xs text-muted-foreground underline"
                    >
                      在{name}创建只读 API Key
                    </a>
                    <Button variant="ghost" size="sm" disabled={saving} onClick={() => edit(null)}>
                      取消
                    </Button>
                    <Button size="sm" disabled={!complete || saving} onClick={() => save(exchange, form)}>
                      {saving && <LoaderCircle className="animate-spin" />}
                      {saving ? "正在验证" : "保存"}
                    </Button>
                  </div>
                </div>
              )}
              {problem?.exchange === exchange && (
                <p className="mx-3.5 mb-3 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
                  {problem.message}
                </p>
              )}
            </div>
          );
        })}
        {configured && (
          <button
            type="button"
            className="flex h-10 w-full items-center justify-between px-3.5 text-left outline-none hover:bg-accent/50 focus-visible:bg-accent/50"
            onClick={() => void api.openHoldings()}
          >
            查看持仓
            <ChevronRight className="size-4 text-muted-foreground" />
          </button>
        )}
      </Group>
    </Section>
  );
}

/** System Settings style: small title above a grouped box, optional footnote below. */
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

/** A single-line utility bar; update feedback never changes its height. */
function AppFooter() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // An event is always newer than the initial fetch below.
    let gotEvent = false;
    const stop = subscribe(
      api.onUpdate((next) => {
        gotEvent = true;
        setError(null);
        setUpdate(next);
      }),
    );
    api
      .getUpdate()
      .then((found) => {
        if (!gotEvent) setUpdate(found);
      })
      .catch(() => {
        if (!gotEvent) setError("暂时无法读取版本信息");
      });
    return stop;
  }, []);

  const state = update?.state;
  const busy =
    state?.kind === "checking" || state?.kind === "downloading" || state?.kind === "restarting";
  const note = error ?? (state ? updateNote(state) : null);
  const act = async () => {
    setError(null);
    try {
      if (!state) setUpdate(await api.getUpdate());
      else if (state.kind === "ready") await api.restartToUpdate();
      else await api.checkUpdate();
    } catch {
      setError("操作未完成，请重试");
    }
  };
  return (
    <footer className="flex h-8 shrink-0 items-center justify-between gap-4 border-t px-5 text-2xs text-muted-foreground">
      <p className="flex shrink-0 items-baseline gap-1.5">
        <span>Candlewick</span>
        {update && <span className="tabular-nums opacity-70">v{update.current}</span>}
      </p>
      {state?.kind === "disabled" ? (
        <span title={note ?? undefined}>更新不可用</span>
      ) : state || error ? (
        <button
          type="button"
          className={cn(
            "-mr-1 flex h-6 min-w-0 items-center gap-1.5 rounded px-1 font-normal outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none",
            state?.kind === "ready" && "text-primary",
          )}
          disabled={busy}
          title={state?.kind === "idle" && state.checked ? "再次检查更新" : note ?? "检查是否有新版本"}
          aria-label={state?.kind === "idle" && state.checked ? "已是最新版本，再次检查更新" : undefined}
          onClick={() => void act()}
        >
          {busy && <LoaderCircle className="size-3 shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
          <span className="truncate">{error ? "操作失败，重试" : state && updateLabel(state)}</span>
        </button>
      ) : null}
      <span role="status" aria-live="polite" className="sr-only">
        {note}
      </span>
    </footer>
  );
}

function updateLabel(state: UpdateState): string {
  switch (state.kind) {
    case "disabled":
      return "更新不可用";
    case "idle":
      return state.checked ? "已是最新版本" : "检查更新";
    case "checking":
      return "检查中…";
    case "downloading":
      return "下载更新中…";
    case "ready":
      return "重启更新";
    case "restarting":
      return "重启中…";
    case "unreachable":
      return "检查失败，重试";
    case "failed":
      return "更新失败，重试";
  }
}

function updateNote(state: UpdateState): string | null {
  switch (state.kind) {
    case "disabled":
      return "此副本不支持应用内更新";
    case "idle":
      return state.checked ? "已是最新版本" : null;
    case "checking":
      return "正在检查更新…";
    case "unreachable":
      return "暂时无法检查更新，请稍后重试";
    case "downloading":
      return `正在下载 ${state.version}…`;
    case "ready":
      return `${state.version} 已下载，重新启动后生效`;
    case "restarting":
      return "正在重新启动…";
    case "failed":
      return `${state.version} 更新失败，请重试`;
  }
}

/** "BTC/USDT", "AAPL 苹果 美股", "腾讯控股 700 港股": the name, then dimmed details. */
function InstrumentName(props: { instrument: Instrument; className?: string; dim?: string }) {
  const { name, detail } = instrumentLabel(props.instrument);
  const market = marketLabel(props.instrument);
  const dim = props.dim ?? "text-muted-foreground";
  return (
    <span className={props.className}>
      <span className="font-medium">{name}</span>
      {detail && <span className={dim}>{detail}</span>}
      {market && <span className={cn("ml-1.5 text-xs", dim)}>{market}</span>}
    </span>
  );
}

/** Searching stocks goes over the network, so it waits for a pause in typing. */
const SEARCH_DELAY_MS = 200;

function InstrumentSearch(props: { existing: Instrument[]; full: boolean; onAdd: (instrument: Instrument) => void }) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResults | null>(null);
  const [loading, setLoading] = useState(false);
  // -1 = follow the first candidate that can still be added.
  const [active, setActive] = useState(-1);
  /** Only the latest search may show its results. */
  const latest = useRef(0);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const run = (text: string, delay: number) => {
    const seq = ++latest.current;
    window.clearTimeout(timer.current);
    setLoading(true);
    timer.current = window.setTimeout(() => {
      api
        .search(text)
        .then((found) => seq === latest.current && setResults(found))
        .catch((e: unknown) => seq === latest.current && setResults({ candidates: [], notes: [String(e)] }))
        .finally(() => seq === latest.current && setLoading(false));
    }, delay);
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
        placeholder={props.full ? `最多 ${MAX_INSTRUMENTS} 个` : "搜索币种或股票代码，如 SOL、AAPL、700"}
        className="h-8 rounded-lg bg-card pl-8 dark:bg-card"
        spellCheck={false}
        autoCorrect="off"
        // Starts loading the pair list before the first keystroke.
        onFocus={() => results || loading || run("", 0)}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(-1);
          run(e.target.value, SEARCH_DELAY_MS);
        }}
        onKeyDown={onKeyDown}
      />
      {query.trim() !== "" && (
        <div className="absolute inset-x-0 top-full z-10 mt-1 overflow-hidden rounded-xl border bg-popover p-1 text-popover-foreground shadow-lg backdrop-blur-xl">
          {options.length === 0 ? (
            loading ? (
              <p className="flex items-center gap-2 px-2.5 py-1.5 text-muted-foreground">
                <LoaderCircle className="size-4 animate-spin" /> 正在搜索…
              </p>
            ) : (
              <div className="space-y-1 px-2.5 py-1.5 text-muted-foreground">
                {(results?.notes.length ? results.notes : ["没有找到"]).map((note) => (
                  <p key={note}>{note}</p>
                ))}
              </div>
            )
          ) : (
            options.map((candidate, index) => {
              const added = taken.has(instrumentId(candidate));
              const highlighted = index === current && !added;
              return (
                <button
                  key={instrumentId(candidate)}
                  type="button"
                  disabled={added}
                  onMouseEnter={() => setActive(index)}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => add(candidate)}
                  className={cn(
                    "flex w-full items-center justify-between gap-2 rounded-md px-2.5 py-1 text-left",
                    highlighted && "bg-primary text-primary-foreground",
                    added && "text-muted-foreground",
                  )}
                >
                  <InstrumentName
                    instrument={candidate}
                    className="min-w-0 truncate"
                    dim={highlighted ? "opacity-80" : "text-muted-foreground"}
                  />
                  <span className="shrink-0 text-xs opacity-70">
                    {added ? "已添加" : candidate.manual ? "手动添加" : ""}
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

const LONGBRIDGE_FIELDS = [
  { key: "appKey", label: "App Key", secret: false },
  { key: "appSecret", label: "App Secret", secret: true },
  { key: "accessToken", label: "Access Token", secret: true },
] as const satisfies readonly { key: keyof LongbridgeKeys; label: string; secret: boolean }[];

/** Credentials for stocks: entered once, checked by logging in. */
function LongbridgeSection() {
  const [state, setState] = useState<Longbridge | null>(null);
  const [editing, setEditing] = useState(false);
  const [keys, setKeys] = useState<LongbridgeKeys>({ appKey: "", appSecret: "", accessToken: "" });
  const [checking, setChecking] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const check = () => {
    setChecking(true);
    setProblem(null);
    api
      .checkLongbridge()
      .then((account) => setState((s) => s && { ...s, account }))
      .catch((e: unknown) => setProblem(String(e)))
      .finally(() => setChecking(false));
  };

  useEffect(() => {
    api
      .getLongbridge()
      .then((found) => {
        setState(found);
        if (found.appKey && !found.account) check();
      })
      .catch((e: unknown) => setProblem(String(e)));
  }, []);

  const save = () => {
    setProblem(null);
    api
      .setLongbridge(keys)
      .then((saved) => {
        setState(saved);
        setEditing(false);
        setKeys({ appKey: "", appSecret: "", accessToken: "" });
        check();
      })
      .catch((e: unknown) => setProblem(String(e)));
  };

  const remove = () => {
    setProblem(null);
    api
      .setLongbridge(null)
      .then(setState)
      .catch((e: unknown) => setProblem(String(e)));
  };

  if (!state) return null;
  const configured = state.appKey !== null && !editing;
  const expires = state.account?.tokenExpires;

  return (
    <Section
      title="长桥"
      footnote={configured ? undefined : "填写长桥 OpenAPI 凭证后，可以添加美股、港股和 A 股。"}
    >
      <Group>
        {configured ? (
          <>
            <div className="flex min-h-10 items-center gap-2 px-3.5 py-2">
              <div className="min-w-0 flex-1 space-y-0.5">
                <p>
                  App Key <span className="font-mono text-xs text-muted-foreground">{state.appKey}</span>
                </p>
                {expires && <p className="text-xs text-muted-foreground">Access Token {fmtDate(expires)}到期，到期前会自动续期</p>}
              </div>
              <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
                更换
              </Button>
              <Button variant="ghost" size="sm" className="hover:text-destructive" onClick={remove}>
                移除
              </Button>
            </div>
            <div className="space-y-1 px-3.5 py-2 text-xs">
              {checking ? (
                <p className="flex items-center gap-2 text-muted-foreground">
                  <LoaderCircle className="size-3.5 animate-spin" /> 正在登录长桥…
                </p>
              ) : state.account ? (
                state.account.markets.map((m) => (
                  <p key={m.market} className="flex gap-2">
                    <span className="w-9 shrink-0 text-muted-foreground">{m.market}</span>
                    <span>{m.packages.length ? m.packages.join("、") : (m.note ?? "无行情权限")}</span>
                  </p>
                ))
              ) : (
                <button type="button" className="text-muted-foreground underline" onClick={check}>
                  查看行情权限
                </button>
              )}
            </div>
          </>
        ) : (
          <div className="space-y-2 px-3.5 py-3">
            {LONGBRIDGE_FIELDS.map((field) => (
              <label key={field.key} className="flex items-center gap-3">
                <span className="w-24 shrink-0 text-muted-foreground">{field.label}</span>
                <Input
                  type={field.secret ? "password" : "text"}
                  value={keys[field.key]}
                  className="h-7 rounded-md font-mono text-xs"
                  spellCheck={false}
                  autoCorrect="off"
                  onChange={(e) => setKeys({ ...keys, [field.key]: e.target.value })}
                />
              </label>
            ))}
            <div className="flex items-center justify-end gap-2 pt-1">
              <a
                href="https://open.longbridge.com/"
                target="_blank"
                rel="noreferrer"
                className="mr-auto text-xs text-muted-foreground underline"
              >
                在长桥开发者中心获取
              </a>
              {state.appKey !== null && (
                <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
                  取消
                </Button>
              )}
              <Button
                size="sm"
                disabled={!keys.appKey.trim() || !keys.appSecret.trim() || !keys.accessToken.trim()}
                onClick={save}
              >
                保存
              </Button>
            </div>
          </div>
        )}
      </Group>
      {problem && <p className="rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">{problem}</p>}
    </Section>
  );
}

/** Epoch seconds → "12 月 25 日", with the year when it isn't this one. */
function fmtDate(secs: number): string {
  const d = new Date(secs * 1000);
  const year = d.getFullYear() === new Date().getFullYear() ? "" : `${d.getFullYear()} 年 `;
  return `${year}${d.getMonth() + 1} 月 ${d.getDate()} 日`;
}
