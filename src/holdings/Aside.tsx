import { cn } from "cn";

import { EXCHANGES, type ExchangeAccount, type HeldPosition } from "@/lib/api";
import { fmtAmount, fmtClock, fmtPct, fmtPrice, fmtSigned, priceDecimals } from "@/lib/format";
import { Spinner } from "@/components/ui/spinner";
import { trend } from "./AssetList";
import { liquidationDistance, returnOnMargin } from "./summary";

const exchangeName = (value: string) => EXCHANGES.find((e) => e.value === value)?.label ?? value;

/** Each exchange: its total, what the day did to it, and where it sits. */
export function Accounts(props: { accounts: ExchangeAccount[] }) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto text-xs">
      {props.accounts.map((account) => {
        const pct =
          account.total !== null && account.change !== null && account.total - account.change > 0
            ? (account.change / (account.total - account.change)) * 100
            : null;
        const widest = Math.max(...account.wallets.map((w) => w.value), 0) || 1;
        return (
          <section key={account.exchange} className="border-b px-3 py-2.5 last:border-b-0">
            <div className="flex items-baseline justify-between gap-2">
              <span className="font-medium">{account.name}</span>
              <span className="font-medium tabular">
                {account.total === null ? <Spinner className="size-3.5 translate-y-0.5" aria-label="读取中" /> : fmtPrice(account.total, 2)}
              </span>
            </div>
            <div className="mt-0.5 flex items-baseline justify-between gap-2 text-2xs text-muted-foreground tabular">
              <span>{account.updated === null ? "正在读取…" : `更新于 ${fmtClock(account.updated)}`}</span>
              {pct !== null && account.change !== null && (
                <span className={trend(pct)}>
                  {fmtPct(pct)} · {fmtSigned(account.change, 2)}
                </span>
              )}
            </div>
            {account.wallets.length > 0 && (
              <ul className="mt-2 space-y-1 tabular">
                {account.wallets.map((wallet) => (
                  <li key={wallet.wallet} className="grid grid-cols-[4.5rem_1fr_auto] items-center gap-2 text-2xs">
                    <span className="truncate text-muted-foreground">{wallet.label}</span>
                    <span className="h-1 overflow-hidden rounded-full bg-fill-strong">
                      <span
                        className="block h-full rounded-full bg-glow opacity-70 transition-[width] duration-500"
                        style={{ width: `${(wallet.value / widest) * 100}%` }}
                      />
                    </span>
                    <span>{fmtPrice(wallet.value, 2)}</span>
                  </li>
                ))}
              </ul>
            )}
            {account.error && (
              <p className="mt-2 text-2xs text-destructive">
                {account.total === null ? account.error : `刷新失败：${account.error}`}
              </p>
            )}
          </section>
        );
      })}
    </div>
  );
}

/** Open derivatives positions, the largest PnL first: a card each. */
export function Positions(props: { positions: HeldPosition[]; severalExchanges: boolean }) {
  if (props.positions.length === 0) {
    return <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">没有合约仓位</div>;
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto text-xs">
      {props.positions.map((p) => {
        const decimals = priceDecimals(p.mark, null);
        const settled = p.pnlAsset === "USDT" || p.pnlAsset === "USDC";
        const pnl = p.pnlUsd ?? (settled ? p.pnl : null);
        const roe = returnOnMargin(p);
        const distance = liquidationDistance(p);
        const facts = [
          p.kind.replace("U 本位", "").replace("币本位", "币本位 "),
          `${p.long ? "多" : "空"}${p.leverage !== null ? ` ${fmtAmount(p.leverage)}x` : ""}`,
          p.isolated ? "逐仓" : "全仓",
          ...(props.severalExchanges ? [exchangeName(p.exchange)] : []),
        ];
        return (
          <section key={`${p.exchange}:${p.symbol}:${p.long}`} className="border-b px-3 py-2.5 last:border-b-0">
            <div className="flex items-baseline justify-between gap-2">
              <span className="truncate font-medium">{p.symbol}</span>
              <span className={cn("font-medium tabular", trend(pnl))}>
                {pnl === null ? `${fmtSigned(p.pnl, 6)} ${p.pnlAsset}` : fmtSigned(pnl, 2)}
              </span>
            </div>
            <div className="mt-0.5 flex items-baseline justify-between gap-2 text-2xs text-muted-foreground tabular">
              <span className="truncate">{facts.join(" · ")}</span>
              {roe !== null && <span className={trend(roe)}>{fmtPct(roe)}</span>}
            </div>
            <dl className="mt-2 grid grid-cols-3 gap-2 text-2xs tabular">
              <Fact label="数量" value={`${fmtAmount(p.size)} ${p.sizeUnit}`} />
              <Fact label="开仓" value={fmtPrice(p.entry, decimals)} />
              <Fact label="标记" value={fmtPrice(p.mark, decimals)} />
            </dl>
            {p.liquidation !== null && distance !== null && (
              <div className="mt-2">
                <div className="flex justify-between text-2xs text-muted-foreground tabular">
                  <span>强平 {fmtPrice(p.liquidation, decimals)}</span>
                  <span className={cn(distance < 0.05 && "text-destructive", distance >= 0.05 && distance < 0.15 && "text-busy")}>
                    距 {(distance * 100).toFixed(1)}%
                  </span>
                </div>
                <div className="mt-1 h-1 overflow-hidden rounded-full bg-fill-strong">
                  <div
                    className={cn(
                      "h-full rounded-full transition-[width] duration-500",
                      distance < 0.05 ? "bg-destructive" : distance < 0.15 ? "bg-busy" : "bg-glow opacity-70",
                    )}
                    style={{ width: `${Math.min(100, distance * 200)}%` }}
                  />
                </div>
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

function Fact(props: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-muted-foreground">{props.label}</dt>
      <dd className="truncate">{props.value}</dd>
    </div>
  );
}
