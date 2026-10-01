import { useEffect, useState, type ReactNode } from "react";
import { LoaderCircle, TriangleAlert } from "lucide-react";
import { cn } from "cn";

import { TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  api,
  subscribe,
  type ExchangeAccount,
  type HeldAsset,
  type HeldPosition,
  type Portfolio,
} from "@/lib/api";
import { direction, fmtAmount, fmtClock, fmtPct, fmtPrice, fmtSigned, priceDecimals } from "@/lib/format";
import { load, store } from "@/lib/prefs";

/** Assets worth less than this (USDT) hide unless asked for. */
const SMALL = 1;

export default function HoldingsApp() {
  const [portfolio, setPortfolio] = useState<Portfolio | null>(null);
  const [hideSmall, setHideSmall] = useState(() =>
    load("holdings.hideSmall", (raw) => (raw === "true" ? true : raw === "false" ? false : undefined), true),
  );
  const [scrolled, setScrolled] = useState(false);

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
      .catch(() => setPortfolio({ accounts: [], total: null, change: null }))
      .finally(() => void api.ready());
    return () => stops.forEach((stop) => stop());
  }, []);

  const toggleSmall = (hide: boolean) => {
    setHideSmall(hide);
    store("holdings.hideSmall", String(hide));
  };

  return (
    <main className="flex h-screen flex-col select-none">
      <TitleBar title="持仓" divider={scrolled} maximizable />
      {!portfolio ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <LoaderCircle className="size-5 animate-spin" />
        </div>
      ) : portfolio.accounts.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center text-muted-foreground">
          <p>在设置的「持仓」里填写交易所的只读 API Key 后，这里会显示持仓。</p>
          <Button size="sm" variant="outline" onClick={() => void api.openSettings()}>
            打开设置
          </Button>
        </div>
      ) : (
        <div
          className="flex-1 space-y-4 overflow-y-auto px-5 pt-2 pb-6"
          onScroll={(e) => setScrolled(e.currentTarget.scrollTop > 0)}
        >
          <Summary portfolio={portfolio} hideSmall={hideSmall} onHideSmall={toggleSmall} />
          {portfolio.accounts.map((account) => (
            <AccountCard key={account.exchange} account={account} hideSmall={hideSmall} />
          ))}
        </div>
      )}
    </main>
  );
}

function Summary(props: { portfolio: Portfolio; hideSmall: boolean; onHideSmall: (hide: boolean) => void }) {
  const { total, change } = props.portfolio;
  return (
    <div className="flex items-end justify-between gap-4">
      <div>
        <p className="text-xs text-muted-foreground">总资产</p>
        <p className="mt-1 flex items-baseline gap-2">
          <span className="text-[30px] leading-none font-semibold tracking-tight tabular">
            {total === null ? "—" : fmtPrice(total, 2)}
          </span>
          <span className="text-xs text-muted-foreground">USDT</span>
        </p>
        {total !== null && change !== null && <Change value={change} base={total - change} className="mt-2" />}
      </div>
      <label className="flex items-center gap-2 text-xs text-muted-foreground">
        隐藏小额资产
        <Switch size="sm" checked={props.hideSmall} onCheckedChange={props.onHideSmall} />
      </label>
    </div>
  );
}

/** "+123.45 · +1.01% 24h", colored by direction. */
function Change(props: { value: number; base: number; className?: string }) {
  const pct = props.base > 0 ? (props.value / props.base) * 100 : 0;
  const sign = direction(props.value);
  return (
    <span
      className={cn(
        "inline-flex rounded-full px-2 py-0.5 text-xs font-medium tabular",
        sign > 0 && "bg-up/12 text-up",
        sign < 0 && "bg-down/12 text-down",
        sign === 0 && "bg-black/5 text-muted-foreground dark:bg-white/8",
        props.className,
      )}
    >
      {fmtSigned(props.value, 2)} · {fmtPct(pct)}
      <span className="ml-1 opacity-70">24h</span>
    </span>
  );
}

function AccountCard(props: { account: ExchangeAccount; hideSmall: boolean }) {
  const { account, hideSmall } = props;
  const holdings = account.holdings;
  const shown = holdings?.assets.filter((a) => !hideSmall || (a.value ?? 0) >= SMALL) ?? [];
  const hidden = (holdings?.assets.length ?? 0) - shown.length;
  return (
    <section className="rounded-2xl border bg-card">
      <header className="flex items-center gap-3 px-4 pt-3">
        <h2 className="font-semibold">{account.name}</h2>
        {holdings && (
          <span className="tabular">
            {fmtPrice(holdings.total, 2)} <span className="text-xs text-muted-foreground">USDT</span>
          </span>
        )}
        {holdings && <Change value={holdings.change} base={holdings.total - holdings.change} />}
        <span className="flex-1" />
        {account.updated !== null && (
          <span className="text-2xs text-muted-foreground tabular">更新于 {fmtClock(account.updated)}</span>
        )}
      </header>
      {holdings && holdings.wallets.length > 0 && (
        <p className="px-4 pt-1 text-xs text-muted-foreground tabular">
          {holdings.wallets.map((w) => `${w.label} ${fmtPrice(w.value, 2)}`).join(" · ")}
        </p>
      )}
      {account.error && (
        <p className="mx-4 mt-2 flex items-start gap-1.5 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
          <TriangleAlert className="mt-px size-3.5 shrink-0" />
          {holdings ? `刷新失败，显示的是上次的数据：${account.error}` : account.error}
        </p>
      )}
      {!holdings && !account.error && (
        <p className="flex items-center gap-2 px-4 py-4 text-muted-foreground">
          <LoaderCircle className="size-4 animate-spin" /> 正在读取持仓…
        </p>
      )}
      {holdings && (
        <div className="space-y-3 px-2 pt-2 pb-3">
          {shown.length > 0 ? (
            <AssetTable assets={shown} />
          ) : (
            holdings.positions.length === 0 && <p className="px-2 py-2 text-muted-foreground">没有资产</p>
          )}
          {hidden > 0 && <p className="px-2 text-xs text-muted-foreground">另有 {hidden} 个小额资产未显示</p>}
          {holdings.positions.length > 0 && <PositionTable positions={holdings.positions} />}
        </div>
      )}
    </section>
  );
}

function Table(props: { head: ReactNode[]; children: ReactNode }) {
  return (
    <table className="w-full table-fixed border-separate border-spacing-x-2 text-right tabular">
      <thead className="text-2xs text-muted-foreground">
        <tr>
          {props.head.map((cell, index) => (
            <th key={index} className={cn("pb-1 font-normal", index === 0 && "text-left")}>
              {cell}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>{props.children}</tbody>
    </table>
  );
}

function AssetTable(props: { assets: HeldAsset[] }) {
  const costs = props.assets.some((a) => a.cost !== null || a.pnl !== null);
  return (
    <Table head={["资产", "数量", "价格", "价值 USDT", "24h", ...(costs ? ["成本 / 浮动盈亏"] : [])]}>
      {props.assets.map((asset) => (
        <tr key={asset.asset} className="align-top">
          <td className="py-1 text-left">
            <span className="font-medium">{asset.asset}</span>
            {asset.wallets.length > 1 && (
              <span className="block truncate text-2xs text-muted-foreground">
                {asset.wallets.map((w) => `${w.label} ${fmtAmount(w.amount)}`).join(" · ")}
              </span>
            )}
            {asset.wallets.length === 1 && (
              <span className="block text-2xs text-muted-foreground">{asset.wallets[0].label}</span>
            )}
          </td>
          <td className="py-1">{fmtAmount(asset.amount)}</td>
          <td className="py-1">{asset.price === null ? "—" : fmtPrice(asset.price, priceDecimals(asset.price, null))}</td>
          <td className="py-1 font-medium">{asset.value === null ? "—" : fmtPrice(asset.value, 2)}</td>
          <td className={cn("py-1", trend(asset.changePct))}>{asset.changePct === null ? "—" : fmtPct(asset.changePct)}</td>
          {costs && (
            <td className="py-1">
              {asset.cost === null ? "—" : fmtPrice(asset.cost, priceDecimals(asset.cost, null))}
              {asset.pnl !== null && (
                <span className={cn("block text-2xs", trend(asset.pnl))}>{fmtSigned(asset.pnl, 2)}</span>
              )}
            </td>
          )}
        </tr>
      ))}
    </Table>
  );
}

function PositionTable(props: { positions: HeldPosition[] }) {
  return (
    <Table head={["合约", "方向", "数量", "开仓价", "标记价", "强平价", "未实现盈亏"]}>
      {props.positions.map((p) => {
        const decimals = priceDecimals(p.mark, null);
        return (
          <tr key={`${p.symbol}:${p.long}`} className="align-top">
            <td className="py-1 text-left">
              <span className="block truncate font-medium">{p.symbol}</span>
              <span className="block text-2xs text-muted-foreground">
                {p.kind}
                {p.isolated && " · 逐仓"}
              </span>
            </td>
            <td className={cn("py-1", p.long ? "text-up" : "text-down")}>
              {p.long ? "多" : "空"}
              {p.leverage !== null && <span className="ml-1 text-2xs opacity-80">{p.leverage}x</span>}
            </td>
            <td className="py-1">
              {fmtAmount(p.size)} <span className="text-2xs text-muted-foreground">{p.sizeUnit}</span>
            </td>
            <td className="py-1">{fmtPrice(p.entry, decimals)}</td>
            <td className="py-1">{fmtPrice(p.mark, decimals)}</td>
            <td className="py-1">{p.liquidation === null ? "—" : fmtPrice(p.liquidation, decimals)}</td>
            <td className={cn("py-1 font-medium", trend(p.pnl))}>
              {fmtSigned(p.pnl, p.pnlAsset === "USDT" || p.pnlAsset === "USDC" ? 2 : 6)}
              <span className="block text-2xs font-normal opacity-80">
                {p.pnlAsset}
                {p.pnlUsd !== null && p.pnlAsset !== "USDT" && ` ≈ ${fmtSigned(p.pnlUsd, 2)} USDT`}
              </span>
            </td>
          </tr>
        );
      })}
    </Table>
  );
}

/** The up or down color for a signed number, none when flat or unknown. */
function trend(value: number | null): string | undefined {
  if (value === null) return undefined;
  const sign = direction(value);
  return sign > 0 ? "text-up" : sign < 0 ? "text-down" : undefined;
}
