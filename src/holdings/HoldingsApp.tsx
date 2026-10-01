import { useEffect, useState, type ReactNode } from "react";
import { LoaderCircle } from "lucide-react";
import { cn } from "cn";

import { ChangeBadge } from "@/components/ChangeBadge";
import { TitleBar } from "@/components/TitleBar";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import {
  api,
  subscribe,
  type ExchangeAccount,
  type HeldAsset,
  type HeldPosition,
  type Portfolio,
} from "@/lib/api";
import { fmtAmount, fmtClock, fmtPct, fmtPrice, fmtSigned, priceDecimals } from "@/lib/format";
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
          <p>在设置的「持仓」里填写交易所的 API Key 后，这里会显示持仓。</p>
          <Button size="sm" onClick={() => void api.openSettings()}>
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
        <p className="text-xs text-muted-foreground">总资产(USDT)</p>
        <p className="mt-1 flex items-baseline gap-3">
          <span className="text-[34px] leading-none font-semibold tracking-tight tabular">
            {total === null ? "—" : fmtPrice(total, 2)}
          </span>
          {total !== null && change !== null && <Change value={change} total={total} />}
        </p>
      </div>
      <Label className="gap-2 text-xs font-normal text-muted-foreground">
        隐藏小额资产
        <Switch size="sm" checked={props.hideSmall} onCheckedChange={props.onHideSmall} />
      </Label>
    </div>
  );
}

/** The 24h change of holdings now worth `total`, as the chart shows a price's. */
function Change(props: { value: number; total: number }) {
  const before = props.total - props.value;
  const pct = before > 0 ? (props.value / before) * 100 : 0;
  return <ChangeBadge pct={pct} amount={fmtSigned(props.value, 2)} span="24h" />;
}

function AccountCard(props: { account: ExchangeAccount; hideSmall: boolean }) {
  const { account, hideSmall } = props;
  const holdings = account.holdings;
  const shown = holdings?.assets.filter((a) => !hideSmall || (a.value ?? 0) >= SMALL) ?? [];
  const hidden = (holdings?.assets.length ?? 0) - shown.length;
  return (
    <section className="rounded-xl border bg-card">
      <header className="flex items-center gap-3 px-4 pt-3">
        <h2 className="font-semibold">{account.name}</h2>
        {holdings && (
          <span className="tabular">
            {fmtPrice(holdings.total, 2)} <span className="text-xs text-muted-foreground">USDT</span>
          </span>
        )}
        {holdings && <Change value={holdings.change} total={holdings.total} />}
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
        <p className="mx-4 mt-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {holdings ? `刷新失败，显示的是上次的数据：${account.error}` : account.error}
        </p>
      )}
      {!holdings && !account.error && (
        <p className="flex items-center gap-2 px-4 py-4 text-muted-foreground">
          <LoaderCircle className="size-4 animate-spin" /> 正在读取持仓…
        </p>
      )}
      {holdings && (
        <div className="space-y-3 px-2 pt-1 pb-2">
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

/** Columns of numbers, compact and aligned right like the chart's lists; the first holds names. */
function Columns(props: { head: string[]; children: ReactNode }) {
  return (
    <Table className="text-xs tabular">
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          {props.head.map((cell, index) => (
            <TableHead
              key={cell}
              className={cn("h-7 px-2 text-2xs font-normal text-muted-foreground", index > 0 && "text-right")}
            >
              {cell}
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>{props.children}</TableBody>
    </Table>
  );
}

/** A row's cells: the name left, numbers right. */
const NAME = "px-2 py-1.5 align-top";
const NUMBER = "px-2 py-1.5 text-right align-top";

function AssetTable(props: { assets: HeldAsset[] }) {
  const costs = props.assets.some((a) => a.cost !== null || a.pnl !== null);
  return (
    <Columns head={["资产", "数量", "价格", "价值 USDT", "24h", ...(costs ? ["成本 / 浮动盈亏"] : [])]}>
      {props.assets.map((asset) => (
        <TableRow key={asset.asset}>
          <TableCell className={NAME}>
            <span className="font-medium">{asset.asset}</span>
            <span className="block max-w-56 truncate text-2xs text-muted-foreground">
              {asset.wallets.length === 1
                ? asset.wallets[0].label
                : asset.wallets.map((w) => `${w.label} ${fmtAmount(w.amount)}`).join(" · ")}
            </span>
          </TableCell>
          <TableCell className={NUMBER}>{fmtAmount(asset.amount)}</TableCell>
          <TableCell className={NUMBER}>
            {asset.price === null ? "—" : fmtPrice(asset.price, priceDecimals(asset.price, null))}
          </TableCell>
          <TableCell className={cn(NUMBER, "font-medium")}>
            {asset.value === null ? "—" : fmtPrice(asset.value, 2)}
          </TableCell>
          <TableCell className={cn(NUMBER, trend(asset.changePct))}>
            {asset.changePct === null ? "—" : fmtPct(asset.changePct)}
          </TableCell>
          {costs && (
            <TableCell className={NUMBER}>
              {asset.cost === null ? "—" : fmtPrice(asset.cost, priceDecimals(asset.cost, null))}
              {asset.pnl !== null && (
                <span className={cn("block text-2xs", trend(asset.pnl))}>{fmtSigned(asset.pnl, 2)}</span>
              )}
            </TableCell>
          )}
        </TableRow>
      ))}
    </Columns>
  );
}

function PositionTable(props: { positions: HeldPosition[] }) {
  return (
    <Columns head={["合约", "方向", "数量", "开仓价", "标记价", "强平价", "未实现盈亏"]}>
      {props.positions.map((p) => {
        const decimals = priceDecimals(p.mark, null);
        const settled = p.pnlAsset === "USDT" || p.pnlAsset === "USDC";
        return (
          <TableRow key={`${p.symbol}:${p.long}`}>
            <TableCell className={NAME}>
              <span className="font-medium">{p.symbol}</span>
              <span className="block text-2xs text-muted-foreground">
                {p.kind}
                {p.isolated && " · 逐仓"}
              </span>
            </TableCell>
            <TableCell className={cn(NUMBER, p.long ? "text-up" : "text-down")}>
              {p.long ? "多" : "空"}
              {p.leverage !== null && <span className="ml-1 text-2xs opacity-80">{p.leverage}x</span>}
            </TableCell>
            <TableCell className={NUMBER}>
              {fmtAmount(p.size)} <span className="text-2xs text-muted-foreground">{p.sizeUnit}</span>
            </TableCell>
            <TableCell className={NUMBER}>{fmtPrice(p.entry, decimals)}</TableCell>
            <TableCell className={NUMBER}>{fmtPrice(p.mark, decimals)}</TableCell>
            <TableCell className={NUMBER}>
              {p.liquidation === null ? "—" : fmtPrice(p.liquidation, decimals)}
            </TableCell>
            <TableCell className={cn(NUMBER, "font-medium", trend(p.pnl, settled ? 2 : 6))}>
              {fmtSigned(p.pnl, settled ? 2 : 6)}
              <span className="block text-2xs font-normal opacity-80">
                {p.pnlAsset}
                {p.pnlUsd !== null && !settled && ` ≈ ${fmtSigned(p.pnlUsd, 2)} USDT`}
              </span>
            </TableCell>
          </TableRow>
        );
      })}
    </Columns>
  );
}

/** The up or down color for a number shown with `decimals`, none when it shows as flat or is unknown. */
function trend(value: number | null, decimals = 2): string | undefined {
  if (value === null) return undefined;
  const sign = Math.sign(Number(value.toFixed(decimals)));
  return sign > 0 ? "text-up" : sign < 0 ? "text-down" : undefined;
}
