import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { ChevronDown, ChevronUp, Languages, Search, X } from "lucide-react";
import { cn } from "cn";

import { Picker } from "@/components/Picker";
import { TitleBar } from "@/components/TitleBar";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { LOCALES, PLATFORM, type Language, type Locale } from "@/lib/i18n";
import {
  api,
  EXCHANGES,
  instrumentId,
  instrumentLabel,
  marketOf,
  MAX_INSTRUMENTS,
  subscribe,
  type AgentStatus,
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
  const { t } = useTranslation();
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
      <TitleBar title={t("settings.title")} divider={scrolled} />

      {!settings ? (
        <div className="flex flex-1 items-center justify-center p-6 text-muted-foreground">
          {error ?? <Spinner className="size-5" aria-label={t("loading")} />}
        </div>
      ) : (
        <div
          className="flex-1 space-y-5 overflow-y-auto px-5 pt-3 pb-6"
          onScroll={(e) => setScrolled(e.currentTarget.scrollTop > 0)}
        >
          <WatchlistSection
            exchange={settings.exchange}
            watchlist={settings.watchlist}
            onChange={(watchlist) =>
              // Pinning an entry takes the bar back from the total holdings.
              void update({
                ...settings,
                watchlist,
                holdingsInBar: settings.holdingsInBar && !watchlist.some((i) => i.pinned),
              })
            }
          />

          <ExchangeSection
            exchange={settings.exchange}
            onChange={(exchange) => void update({ ...settings, exchange })}
          />

          <ApiKeysSection
            holdingsInBar={settings.holdingsInBar}
            concealHoldings={settings.concealHoldings}
            onChange={(change) => void update({ ...settings, ...change })}
          />

          <LongbridgeSection />

          <Section title={t("settings.display.title")}>
            <Group>
              <SwitchRow
                label={t("settings.display.showSymbol", PLATFORM)}
                checked={settings.showSymbol}
                onChange={(showSymbol) => void update({ ...settings, showSymbol })}
              />
              <SwitchRow
                label={t("settings.display.showChange", PLATFORM)}
                checked={settings.showChange}
                onChange={(showChange) => void update({ ...settings, showChange })}
              />
              <SwitchRow
                label={t("settings.display.redUp")}
                checked={settings.colorScheme === "redUp"}
                onChange={(redUp) =>
                  void update({ ...settings, colorScheme: redUp ? "redUp" : "greenUp" })
                }
              />
            </Group>
          </Section>

          <Section title={t("settings.general.title")}>
            <Group>
              <LanguageRow
                language={settings.language}
                onChange={(language) => void update({ ...settings, language })}
              />
              <SwitchRow
                label={t("settings.general.loginItem", PLATFORM)}
                checked={loginItem ?? false}
                disabled={loginItem === null}
                onChange={(enabled) => void toggleLoginItem(enabled)}
              />
              <SwitchRow
                label={t("settings.general.autoUpdate")}
                checked={settings.autoUpdate}
                onChange={(autoUpdate) => void update({ ...settings, autoUpdate })}
              />
              <SwitchRow
                label={t("settings.general.agentAccess")}
                detail={settings.agentAccess && <AgentNote />}
                checked={settings.agentAccess}
                onChange={(agentAccess) => void update({ ...settings, agentAccess })}
              />
            </Group>
          </Section>

          {error && (
            <Problem>{error}</Problem>
          )}
        </div>
      )}

      <AppFooter />
    </main>
  );
}

function WatchlistSection(props: {
  exchange: Exchange;
  watchlist: Instrument[];
  onChange: (watchlist: Instrument[]) => void;
}) {
  const { t } = useTranslation();
  const { watchlist, onChange } = props;
  const move = (index: number, delta: number) => {
    const next = [...watchlist];
    const [moved] = next.splice(index, 1);
    next.splice(index + delta, 0, moved);
    onChange(next);
  };
  return (
    <Section title={t("settings.watchlist.title")} footnote={t("settings.watchlist.footnote", PLATFORM)}>
      <InstrumentSearch
        // Another exchange clears the search: its results were the old exchange's pairs.
        key={props.exchange}
        existing={watchlist}
        full={watchlist.length >= MAX_INSTRUMENTS}
        onAdd={(added) => onChange([...watchlist, { ...added, pinned: watchlist.length === 0 }])}
      />
      <Group>
        {watchlist.length === 0 ? (
          <p className="px-3.5 py-6 text-center text-muted-foreground">{t("settings.watchlist.empty")}</p>
        ) : (
          watchlist.map((item, index) => (
            <div key={instrumentId(item)} className="flex h-10 items-center gap-0.5 pr-1.5 pl-3.5">
              <InstrumentName instrument={item} className="min-w-0 flex-1 truncate" />
              <Label className="mr-2 gap-2 text-xs font-normal text-muted-foreground">
                {t("settings.watchlist.bar", PLATFORM)}
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
                aria-label={t("settings.watchlist.moveUp")}
                className="text-muted-foreground"
                disabled={index === 0}
                onClick={() => move(index, -1)}
              >
                <ChevronUp />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("settings.watchlist.moveDown")}
                className="text-muted-foreground"
                disabled={index === watchlist.length - 1}
                onClick={() => move(index, 1)}
              >
                <ChevronDown />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("settings.watchlist.remove", { name: instrumentLabel(item).name })}
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

/** A choice in a settings row, drawn as each system draws one: a borderless
 * pop-up button on macOS, a combo box on Windows. */
const ROW_PICKER = __WINDOWS__
  ? "flex h-7 min-w-24 items-center justify-between gap-2 rounded-md border border-input bg-transparent pr-2 pl-2.5 hover:bg-muted dark:bg-input/30"
  : "-mr-1.5 flex h-7 items-center gap-1 rounded-md px-2 hover:bg-accent";

function PickerRow<T extends string>(props: {
  label: ReactNode;
  /** The label in words, for screen readers. */
  name: string;
  value: T;
  options: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  const current = props.options.find((o) => o.value === props.value);
  return (
    <div className="flex min-h-10 items-center justify-between gap-4 px-3.5 py-2">
      <span className="flex items-center gap-2">{props.label}</span>
      <Picker
        label={props.name}
        value={props.value}
        options={props.options}
        onChange={(value) => {
          const picked = props.options.find((o) => o.value === value);
          if (picked) props.onChange(picked.value);
        }}
        className={ROW_PICKER}
      >
        <span>{current?.label}</span>
      </Picker>
    </div>
  );
}

/** Which exchange the pairs come from. */
function ExchangeSection(props: { exchange: Exchange; onChange: (exchange: Exchange) => void }) {
  const { t } = useTranslation();
  const label = t("settings.crypto.exchange");
  return (
    <Section title={t("settings.crypto.title")} footnote={t("settings.crypto.footnote")}>
      <Group>
        <PickerRow
          label={label}
          name={label}
          value={props.exchange}
          options={EXCHANGES.map((value) => ({ value, label: t(`common.provider.${value}`) }))}
          onChange={props.onChange}
        />
      </Group>
    </Section>
  );
}

/** Each language by its own name, whatever the page speaks. */
const LANGUAGES = (Object.keys(LOCALES) as Locale[]).map((value) => ({ value, label: LOCALES[value] }));

/** The language the app speaks; marked by the universal icon, so it can be
 * found in a language one can't read. */
function LanguageRow(props: { language: Language; onChange: (language: Language) => void }) {
  const { t } = useTranslation();
  const label = t("settings.general.language");
  return (
    <PickerRow
      label={
        <>
          <Languages className="size-3.5 text-muted-foreground" aria-hidden />
          {label}
        </>
      }
      name={label}
      value={props.language}
      options={[{ value: "system", label: t("settings.general.system") }, ...LANGUAGES]}
      onChange={props.onChange}
    />
  );
}

/** Where each exchange lets one create an API key, in the page's language
 * where the site has it. */
function apiPage(exchange: Exchange, locale: string): string {
  switch (exchange) {
    case "binance":
      return `https://www.binance.com/${locale === "zh-CN" ? "zh-CN" : locale === "ja" ? "ja" : "en"}/my/settings/api-management`;
    case "bybit":
      return "https://www.bybit.com/app/user/api-management";
    case "okx":
      return `https://www.okx.com/${locale === "zh-CN" ? "zh-hans/" : ""}account/my-api`;
  }
}

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
function ApiKeysSection(props: {
  holdingsInBar: boolean;
  concealHoldings: boolean;
  onChange: (change: Partial<Pick<Settings, "holdingsInBar" | "concealHoldings">>) => void;
}) {
  const { t, i18n } = useTranslation();
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
    <Section title={t("settings.holdings.title")} footnote={t("settings.holdings.footnote")}>
      <Group>
        {keys.map(({ exchange, key }) => {
          const name = t(`common.provider.${exchange}`);
          const open = editing === exchange;
          const fields = keyFields(exchange);
          const complete = fields.every(({ field }) => (form[field] ?? "").trim() !== "");
          return (
            <div key={exchange}>
              <div className="flex min-h-10 items-center gap-2 px-3.5 py-2">
                <span className="w-16 shrink-0 truncate">{name}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground">
                  {key ?? (open ? "" : t("settings.holdings.notSet"))}
                </span>
                {!open && (
                  <Button variant="ghost" size="sm" disabled={saving} onClick={() => edit(exchange)}>
                    {key ? t("settings.replace") : t("settings.add")}
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
                    {t("settings.remove")}
                  </Button>
                )}
              </div>
              {open && (
                <div className="space-y-3 px-3.5 pb-3">
                  <FieldGroup className="gap-2.5">
                    {fields.map(({ field, label, secret }) => (
                      <CredentialField
                        key={field}
                        label={label}
                        secret={secret}
                        value={form[field] ?? ""}
                        disabled={saving}
                        onChange={(value) => setForm({ ...form, [field]: value })}
                      />
                    ))}
                  </FieldGroup>
                  <div className="flex items-center justify-end gap-2">
                    <a
                      href={apiPage(exchange, i18n.language)}
                      target="_blank"
                      rel="noreferrer"
                      className="mr-auto text-xs text-muted-foreground underline"
                    >
                      {t("settings.holdings.create", { exchange: name })}
                    </a>
                    <Button variant="ghost" size="sm" disabled={saving} onClick={() => edit(null)}>
                      {t("settings.cancel")}
                    </Button>
                    <Button size="sm" disabled={!complete || saving} onClick={() => save(exchange, form)}>
                      {saving && <Spinner aria-label={t("settings.holdings.verifying")} />}
                      {t("settings.save")}
                    </Button>
                  </div>
                </div>
              )}
              {problem?.exchange === exchange && (
                <Problem className="mx-3.5 mb-3 w-auto">{problem.message}</Problem>
              )}
            </div>
          );
        })}
        {configured && (
          <>
            <SwitchRow
              label={t("settings.holdings.inBar", PLATFORM)}
              detail={<p className="text-xs text-muted-foreground">{t("settings.holdings.inBarDetail")}</p>}
              checked={props.holdingsInBar}
              onChange={(holdingsInBar) => props.onChange({ holdingsInBar })}
            />
            <SwitchRow
              label={t("settings.holdings.conceal")}
              detail={<p className="text-xs text-muted-foreground">{t("settings.holdings.concealDetail")}</p>}
              checked={props.concealHoldings}
              onChange={(concealHoldings) => props.onChange({ concealHoldings })}
            />
            <div className="flex min-h-10 items-center justify-between gap-2 px-3.5 py-2">
              <span>{t("settings.holdings.window")}</span>
              <Button variant="ghost" size="sm" onClick={() => void api.openHoldings()}>
                {t("settings.holdings.open")}
              </Button>
            </div>
          </>
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

/** Which agents can read the app's data now. */
function AgentNote() {
  const { t, i18n } = useTranslation();
  const [status, setStatus] = useState<AgentStatus | null>(null);

  useEffect(() => {
    // An event is always newer than the fetch.
    let gotEvent = false;
    const stop = subscribe(
      api.onAgent((next) => {
        gotEvent = true;
        setStatus(next);
      }),
    );
    api
      .getAgent()
      .then((found) => {
        if (!gotEvent) setStatus(found);
      })
      .catch(() => {});
    return stop;
  }, []);

  if (!status?.on) return null;
  const agents = new Intl.ListFormat(i18n.language, { type: "conjunction" }).format(status.agents);
  const text = status.error
    ? status.error
    : status.agents.length
      ? t("settings.general.agentsCanRead", { agents })
      : t("settings.general.noAgents");
  return <p className={cn("text-xs", status.error ? "text-destructive" : "text-muted-foreground")}>{text}</p>;
}

/** One line of a credentials form. */
function CredentialField(props: {
  label: string;
  secret: boolean;
  value: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  const id = useId();
  return (
    <Field className="gap-1">
      <FieldLabel htmlFor={id} className="text-xs font-normal text-muted-foreground">
        {props.label}
      </FieldLabel>
      <Input
        id={id}
        type={props.secret ? "password" : "text"}
        value={props.value}
        className="h-7 rounded-md font-mono text-xs"
        spellCheck={false}
        autoCorrect="off"
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.target.value)}
      />
    </Field>
  );
}

/** Why something the user asked for didn't happen. */
function Problem(props: { className?: string; children: ReactNode }) {
  return (
    <Alert
      variant="destructive"
      className={cn("border-transparent bg-destructive/10 px-3 py-2 text-xs", props.className)}
    >
      {props.children}
    </Alert>
  );
}

function SwitchRow(props: {
  label: string;
  /** Under the label. */
  detail?: ReactNode;
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
        {props.detail}
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
  const { t } = useTranslation();
  const [update, setUpdate] = useState<Update | null>(null);
  /** What went wrong, as a message of the update section. */
  const [error, setError] = useState<"versionUnavailable" | "actionFailed" | null>(null);

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
        if (!gotEvent) setError("versionUnavailable");
      });
    return stop;
  }, []);

  const state = update?.state;
  const busy =
    state?.kind === "checking" || state?.kind === "downloading" || state?.kind === "restarting";
  const note = error ? t(`settings.update.${error}`) : state ? updateNote(state, t) : null;
  const act = async () => {
    setError(null);
    try {
      if (!state) setUpdate(await api.getUpdate());
      else if (state.kind === "ready") await api.restartToUpdate();
      else await api.checkUpdate();
    } catch {
      setError("actionFailed");
    }
  };
  return (
    <footer className="flex h-8 shrink-0 items-center justify-between gap-4 border-t px-5 text-2xs text-muted-foreground">
      <p className="flex shrink-0 items-baseline gap-1.5">
        <span>Candlewick</span>
        {update && <span className="tabular-nums opacity-70">v{update.current}</span>}
      </p>
      {state?.kind === "disabled" ? (
        <span title={note ?? undefined}>{t("settings.update.unavailable")}</span>
      ) : state || error ? (
        <button
          type="button"
          className={cn(
            "-mr-1 flex h-6 min-w-0 items-center gap-1.5 rounded px-1 font-normal outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none",
            state?.kind === "ready" && "text-primary",
          )}
          disabled={busy}
          title={
            state?.kind === "idle" && state.checked
              ? t("settings.update.checkAgain")
              : (note ?? t("settings.update.checkForNew"))
          }
          aria-label={state?.kind === "idle" && state.checked ? t("settings.update.upToDateCheckAgain") : undefined}
          onClick={() => void act()}
        >
          {busy && <Spinner className="size-3 motion-reduce:animate-none" aria-hidden="true" />}
          <span className="truncate">{error ? t("settings.update.retry") : state && updateLabel(state, t)}</span>
        </button>
      ) : null}
      <span role="status" aria-live="polite" className="sr-only">
        {note}
      </span>
    </footer>
  );
}

function updateLabel(state: UpdateState, t: TFunction): string {
  switch (state.kind) {
    case "disabled":
      return t("settings.update.unavailable");
    case "idle":
      return state.checked ? t("settings.update.upToDate") : t("settings.update.check");
    case "checking":
      return t("settings.update.checking");
    case "downloading":
      return t("settings.update.downloading");
    case "ready":
      return t("settings.update.restartToUpdate");
    case "restarting":
      return t("settings.update.restarting");
    case "unreachable":
      return t("settings.update.checkFailed");
    case "failed":
      return t("settings.update.updateFailed");
  }
}

function updateNote(state: UpdateState, t: TFunction): string | null {
  switch (state.kind) {
    case "disabled":
      return t("settings.update.noteDisabled");
    case "idle":
      return state.checked ? t("settings.update.upToDate") : null;
    case "checking":
      return t("settings.update.noteChecking");
    case "unreachable":
      return t("settings.update.noteUnreachable");
    case "downloading":
      return t("settings.update.noteDownloading", { version: state.version });
    case "ready":
      return t("settings.update.noteReady", { version: state.version });
    case "restarting":
      return t("settings.update.noteRestarting");
    case "failed":
      return t("settings.update.noteFailed", { version: state.version });
  }
}

/** "BTC/USDT", "AAPL Apple US", "腾讯控股 700 港股": the name, then dimmed details. */
function InstrumentName(props: { instrument: Instrument; className?: string; dim?: string }) {
  const { t } = useTranslation();
  const { name, detail } = instrumentLabel(props.instrument);
  const market = marketOf(props.instrument);
  const dim = props.dim ?? "text-muted-foreground";
  return (
    <span className={props.className}>
      <span className="font-medium">{name}</span>
      {detail && <span className={dim}>{detail}</span>}
      {market && <span className={cn("ml-1.5 text-xs", dim)}>{t(`market.${market}`)}</span>}
    </span>
  );
}

/** Searching stocks goes over the network, so it waits for a pause in typing. */
const SEARCH_DELAY_MS = 200;

function InstrumentSearch(props: { existing: Instrument[]; full: boolean; onAdd: (instrument: Instrument) => void }) {
  const { t } = useTranslation();
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
        placeholder={
          props.full ? t("settings.watchlist.full", { max: MAX_INSTRUMENTS }) : t("settings.watchlist.search")
        }
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
                <Spinner aria-hidden /> {t("settings.watchlist.searching")}
              </p>
            ) : (
              <div className="space-y-1 px-2.5 py-1.5 text-muted-foreground">
                {(results?.notes.length ? results.notes : [t("settings.watchlist.noResults")]).map((note) => (
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
                    {added ? t("settings.watchlist.added") : candidate.manual ? t("settings.watchlist.manual") : ""}
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
  const { t } = useTranslation();
  const source = t("common.provider.longbridge");
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

  return (
    <Section title={source} footnote={configured ? undefined : t("settings.stocks.footnote")}>
      <Group>
        {configured ? (
          <>
            <div className="flex min-h-10 items-center gap-2 px-3.5 py-2">
              <p className="min-w-0 flex-1">
                App Key <span className="font-mono text-xs text-muted-foreground">{state.appKey}</span>
              </p>
              <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
                {t("settings.replace")}
              </Button>
              <Button variant="ghost" size="sm" className="hover:text-destructive" onClick={remove}>
                {t("settings.remove")}
              </Button>
            </div>
            <div className="px-3.5 py-2 text-xs">
              {checking ? (
                <p className="flex items-center gap-2 text-muted-foreground">
                  <Spinner className="size-3.5" aria-hidden /> {t("settings.stocks.signingIn", { source })}
                </p>
              ) : state.account ? (
                <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
                  {state.account.markets.map((m) => (
                    <div key={m.market} className="contents">
                      <dt className="text-muted-foreground">{t(`market.${m.market}`)}</dt>
                      <dd>{m.packages.length ? m.packages.join(" · ") : (m.note ?? t("settings.stocks.noAccess"))}</dd>
                    </div>
                  ))}
                </dl>
              ) : (
                <button type="button" className="text-muted-foreground underline" onClick={check}>
                  {t("settings.stocks.checkAccess")}
                </button>
              )}
            </div>
          </>
        ) : (
          <div className="space-y-3 px-3.5 py-3">
            <FieldGroup className="gap-2.5">
              {LONGBRIDGE_FIELDS.map((field) => (
                <CredentialField
                  key={field.key}
                  label={field.label}
                  secret={field.secret}
                  value={keys[field.key]}
                  onChange={(value) => setKeys({ ...keys, [field.key]: value })}
                />
              ))}
            </FieldGroup>
            <div className="flex items-center justify-end gap-2">
              <a
                href="https://open.longbridge.com/"
                target="_blank"
                rel="noreferrer"
                className="mr-auto text-xs text-muted-foreground underline"
              >
                {t("settings.stocks.getKeys", { source })}
              </a>
              {state.appKey !== null && (
                <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
                  {t("settings.cancel")}
                </Button>
              )}
              <Button
                size="sm"
                disabled={!keys.appKey.trim() || !keys.appSecret.trim() || !keys.accessToken.trim()}
                onClick={save}
              >
                {t("settings.save")}
              </Button>
            </div>
          </div>
        )}
      </Group>
      {problem && <Problem>{problem}</Problem>}
    </Section>
  );
}
