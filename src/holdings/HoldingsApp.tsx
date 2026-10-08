import { useEffect, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { KeyRound } from "lucide-react";
import { cn } from "cn";

import { ChangeBadge } from "@/components/ChangeBadge";
import { PillTabs } from "@/components/PillTabs";
import { TITLE_INSET, TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { api, subscribe, type Portfolio } from "@/lib/api";
import { fmtClock, fmtPrice, fmtSigned } from "@/lib/format";
import { load, store } from "@/lib/prefs";
import { Allocation } from "./Allocation";
import { Accounts, Positions } from "./Aside";
import { AssetList, trend } from "./AssetList";
import { allocation, figures, insight, listed, type Sort } from "./summary";

type Tab = "accounts" | "positions";

const SORTS = [
  { value: "value", label: "holdings.byValue" },
  { value: "change", label: "holdings.byChange" },
] as const satisfies readonly { value: Sort; label: string }[];

/** Numbers this recent are live: the lit dot in the title bar and on the bar. */
const LIVE_MS = 20_000;

export default function HoldingsApp() {
  const { t } = useTranslation();
  const [portfolio, setPortfolio] = useState<Portfolio | null>(null);
  const [sort, setSort] = useState<Sort>(() => load("holdings.sort", (raw) => SORTS.find((s) => s.value === raw)?.value, "value"));
  const [showSmall, setShowSmall] = useState(() => load("holdings.showSmall", (raw) => raw === "true", false));
  const [tab, setTab] = useState<Tab>(() => load("holdings.tab", (raw) => (raw === "positions" ? raw : undefined), "accounts"));
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    // An event is always newer than the initial fetch below.
    let gotEvent = false;
    const stops = [
      subscribe(
        api.onPortfolio((next) => {
          gotEvent = true;
          setPortfolio(next);
        }),
      ),
      subscribe(api.onSettings((settings) => (document.documentElement.dataset.scheme = settings.colorScheme))),
    ];
    api
      .getSettings()
      .then((settings) => (document.documentElement.dataset.scheme = settings.colorScheme))
      .catch(() => {});
    api
      .getPortfolio()
      .then((found) => {
        if (!gotEvent) setPortfolio(found);
      })
      .catch(() => setPortfolio({ accounts: [], total: null, change: null, assets: [], positions: [] }))
      .finally(() => void api.ready());
    return () => stops.forEach((stop) => stop());
  }, []);

  // Freshness is a matter of time, not only of data.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const pickSort = (next: Sort) => {
    setSort(next);
    store("holdings.sort", next);
  };
  const pickTab = (next: Tab) => {
    setTab(next);
    store("holdings.tab", next);
  };
  const toggleSmall = () => {
    setShowSmall(!showSmall);
    store("holdings.showSmall", String(!showSmall));
  };

  const latest = portfolio ? Math.max(0, ...portfolio.accounts.map((a) => a.updated ?? 0)) : 0;
  const live = latest > 0 && now - latest < LIVE_MS;
  const total = portfolio?.total ?? null;
  const several = (portfolio?.accounts.length ?? 0) > 1;

  return (
    <main className="flex h-screen flex-col select-none">
      <TitleBar className={cn(TITLE_INSET, "gap-2", !__WINDOWS__ && "pr-2")} maximizable>
        <span className="text-sm font-semibold">{t("holdings.title")}</span>
        <span className="flex-1" />
        {portfolio && portfolio.accounts.length > 0 && <Freshness portfolio={portfolio} live={live} />}
        <Button
          variant="ghost"
          size="icon-sm"
          title={t("holdings.manageKeys")}
          aria-label={t("holdings.manageKeys")}
          className="text-muted-foreground"
          onClick={() => void api.openSettings()}
        >
          <KeyRound />
        </Button>
      </TitleBar>

      {!portfolio ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <Spinner className="size-5" aria-label={t("loading")} />
        </div>
      ) : portfolio.accounts.length === 0 ? (
        <NoKeys />
      ) : (
        <div className="flex min-h-0 flex-1">
          <section className="flex min-w-0 flex-1 flex-col gap-3 pb-4">
            <Hero portfolio={portfolio} />
            <Allocation shares={allocation(portfolio.assets, total ?? 0, t)} live={live} />

            <div className="flex h-7 items-center gap-2 px-4">
              <PillTabs
                label={t("holdings.sort")}
                value={sort}
                options={SORTS.map((o) => ({ value: o.value, label: t(o.label) }))}
                onChange={pickSort}
              />
            </div>

            <Assets portfolio={portfolio} sort={sort} showSmall={showSmall} onToggleSmall={toggleSmall} several={several} />

            <Tiles portfolio={portfolio} />
          </section>

          <aside className="flex w-70 shrink-0 flex-col border-l pt-2">
            <div className="px-3 pb-2.5">
              <PillTabs
                label={t("holdings.accountsAndPositions")}
                value={tab}
                options={[
                  { value: "accounts", label: t("common.holdings.accounts") },
                  {
                    value: "positions",
                    label: portfolio.positions.length
                      ? t("holdings.positionsCount", { n: portfolio.positions.length })
                      : t("common.holdings.positions"),
                  },
                ]}
                onChange={pickTab}
                className="w-full"
              />
            </div>
            {tab === "accounts" ? (
              <Accounts accounts={portfolio.accounts} />
            ) : (
              <Positions positions={portfolio.positions} severalExchanges={several} />
            )}
          </aside>
        </div>
      )}
    </main>
  );
}

/** The total, its day, and one line on what did it; the number flashes as it moves. */
function Hero(props: { portfolio: Portfolio }) {
  const { t } = useTranslation();
  const { total, change } = props.portfolio;
  const before = total !== null && change !== null ? total - change : null;
  const pct = before !== null && before > 0 && change !== null ? (change / before) * 100 : null;
  const line = insight(props.portfolio, t);
  return (
    <div className="px-5 pt-2">
      <div className="flex items-baseline gap-3">
        {total === null ? (
          <span className="h-8.5 w-56 animate-pulse rounded-lg bg-fill-strong" />
        ) : (
          <span
            // Remounted on every new total so the flash replays.
            key={total}
            className="text-[34px] leading-none font-semibold tracking-tight tabular animate-[price-flash_1s_ease-out]"
            style={{ "--flash": (change ?? 0) < 0 ? "var(--down)" : "var(--up)" } as CSSProperties}
          >
            {fmtPrice(total, 2)}
            <span className="ml-1.5 text-sm font-medium tracking-normal text-muted-foreground">USDT</span>
          </span>
        )}
        {total !== null && change !== null && pct !== null && (
          <ChangeBadge pct={pct} amount={fmtSigned(change, 2)} span="24h" />
        )}
      </div>
      <p className="mt-2 h-4 text-xs text-muted-foreground tabular">
        {line ?? (total === null ? t("holdings.readingAccounts") : "")}
      </p>
    </div>
  );
}

function Assets(props: {
  portfolio: Portfolio;
  sort: Sort;
  showSmall: boolean;
  onToggleSmall: () => void;
  several: boolean;
}) {
  const { assets, total } = props.portfolio;
  const { shown, small } = listed(assets, props.sort, props.showSmall);
  if (total === null) {
    return <div className="panel mx-4 min-h-0 flex-1" />;
  }
  return (
    <AssetList
      assets={shown}
      all={assets}
      total={total}
      sort={props.sort}
      severalExchanges={props.several}
      small={{
        count: small.length,
        value: small.reduce((sum, a) => sum + (a.value ?? 0), 0),
        shown: props.showSmall,
        onToggle: props.onToggleSmall,
      }}
    />
  );
}

/** The four figures that say how the money is placed, as the chart's day statistics. */
function Tiles(props: { portfolio: Portfolio }) {
  const { t } = useTranslation();
  const { total, change } = props.portfolio;
  const f = figures(props.portfolio);
  const share = (value: number) => (total && total > 0 ? ` · ${Math.round((value / total) * 100)}%` : "");
  const items: { label: string; value: string | null; tone?: string }[] = [
    { label: t("common.holdings.dayPnl"), value: change === null ? null : fmtSigned(change, 2), tone: trend(change) },
    { label: t("holdings.crypto"), value: total === null ? null : `${fmtPrice(f.risk, 2)}${share(f.risk)}` },
    { label: t("holdings.stablecoins"), value: total === null ? null : `${fmtPrice(f.cash, 2)}${share(f.cash)}` },
    {
      label: t("holdings.unrealized"),
      value:
        total === null ? null : f.positionsPnl === null ? t("holdings.noPositions") : fmtSigned(f.positionsPnl, 2),
      tone: trend(f.positionsPnl),
    },
  ];
  return (
    <dl className="grid grid-cols-4 gap-2 px-4">
      {items.map((item) => (
        <div key={item.label} className="min-w-0 rounded-xl bg-fill px-3 py-2">
          <dt className="text-2xs text-muted-foreground">{item.label}</dt>
          <dd className={cn("mt-0.5 truncate font-medium tabular", item.tone)}>{item.value ?? "—"}</dd>
        </div>
      ))}
    </dl>
  );
}

/** When the numbers are from, and whether they still move: like the chart's live badge. */
function Freshness(props: { portfolio: Portfolio; live: boolean }) {
  const { t } = useTranslation();
  const { accounts } = props.portfolio;
  const failed = accounts.find((a) => a.error !== null);
  const loading = accounts.some((a) => a.updated === null && a.error === null);
  const latest = Math.max(0, ...accounts.map((a) => a.updated ?? 0));
  const text = failed
    ? t("common.holdings.readFailed", { exchange: t(`common.provider.${failed.exchange}`) })
    : loading
      ? t("common.holdings.reading")
      : latest > 0
        ? t("common.holdings.updated", { time: fmtClock(latest) })
        : "";
  return (
    <span className="flex items-center gap-1.5 rounded-full px-2 text-xs text-muted-foreground tabular" title={failed?.error ?? undefined}>
      <span className="relative flex size-2">
        {props.live && !failed && <span className="absolute inset-0 animate-ping rounded-full bg-live opacity-60" />}
        <span
          className={cn(
            "relative size-2 rounded-full",
            failed ? "bg-destructive" : loading ? "bg-busy" : props.live ? "bg-live" : "bg-faint",
          )}
        />
      </span>
      {text}
    </span>
  );
}

function NoKeys() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 px-8 pb-8 text-center">
      <div className="space-y-1.5">
        <p className="text-base font-semibold">{t("holdings.noKeys")}</p>
        <p className="max-w-xs text-xs text-balance text-muted-foreground">{t("holdings.noKeysDetail")}</p>
      </div>
      <Button size="sm" onClick={() => void api.openSettings()}>
        {t("holdings.openSettings")}
      </Button>
    </div>
  );
}
