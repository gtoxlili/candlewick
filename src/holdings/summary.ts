// What the holdings window says about a portfolio: how the money splits,
// what moved it, and the figures in the tiles. Pure functions over the
// app's portfolio, so the page only lays them out.

import type { HeldAsset, HeldPosition, Portfolio } from "@/lib/api";
import { fmtSigned } from "@/lib/format";

/** Assets worth less than this (USDT) are "small": folded away by default. */
export const SMALL = 1;

/** One stretch of the allocation bar. */
export interface Share {
  label: string;
  value: number;
  /** Of the total, 0–100. */
  pct: number;
  /** The CSS color: `var(--series-1)` … `var(--series-cash)`. */
  color: string;
}

/** The largest holdings get a series color each; the rest fold together, cash last. */
const SERIES = 5;

/** How the total splits: the largest assets, 其他, then 稳定币. */
export function allocation(assets: HeldAsset[], total: number): Share[] {
  if (!(total > 0)) return [];
  const priced = assets.filter((a): a is HeldAsset & { value: number } => a.value !== null && a.value > 0);
  const risk = priced.filter((a) => !a.stable);
  const shares: Share[] = risk.slice(0, SERIES).map((a, i) => ({
    label: a.asset,
    value: a.value,
    pct: (a.value / total) * 100,
    color: `var(--series-${i + 1})`,
  }));
  const rest = risk.slice(SERIES).reduce((sum, a) => sum + a.value, 0);
  if (rest > 0) shares.push({ label: "其他", value: rest, pct: (rest / total) * 100, color: "var(--series-rest)" });
  const cash = priced.filter((a) => a.stable).reduce((sum, a) => sum + a.value, 0);
  if (cash > 0) shares.push({ label: "稳定币", value: cash, pct: (cash / total) * 100, color: "var(--series-cash)" });
  return shares;
}

/** The series color of an asset in the list, matching the allocation bar. */
export function seriesColor(asset: HeldAsset, assets: HeldAsset[]): string {
  if (asset.stable) return "var(--series-cash)";
  const rank = assets.filter((a) => !a.stable && a.value !== null && a.value > 0).indexOf(asset);
  return rank >= 0 && rank < SERIES ? `var(--series-${rank + 1})` : "var(--series-rest)";
}

export interface Figures {
  /** Value in assets that move with the market. */
  risk: number;
  /** Value in stablecoins. */
  cash: number;
  /** Unrealized PnL over the open positions, in USDT; null without any. */
  positionsPnl: number | null;
  /** What the 24h moves made of the positions, in USDT. */
  positionsChange: number;
}

export function figures(portfolio: Portfolio): Figures {
  const priced = portfolio.assets.filter((a) => a.value !== null);
  const sum = (list: HeldAsset[]) => list.reduce((total, a) => total + (a.value ?? 0), 0);
  const positions = portfolio.positions;
  return {
    risk: sum(priced.filter((a) => !a.stable)),
    cash: sum(priced.filter((a) => a.stable)),
    positionsPnl: positions.length
      ? positions.reduce((total, p) => total + (p.pnlUsd ?? (p.pnlAsset === "USDT" ? p.pnl : 0)), 0)
      : null,
    positionsChange: positions.reduce((total, p) => total + (p.change ?? 0), 0),
  };
}

/**
 * One line under the total: what moved the money today and how much of it
 * is cash. "BTC +1,320.20 · ETH −210.60 · 合约 +755.10 · 稳定币 26%".
 */
export function insight(portfolio: Portfolio): string | null {
  const { total } = portfolio;
  if (total === null || !(total > 0)) return null;
  const movers = portfolio.assets
    .filter((a) => !a.stable && a.change !== null && Math.abs(a.change) >= 0.005)
    .sort((a, b) => Math.abs(b.change ?? 0) - Math.abs(a.change ?? 0))
    .slice(0, 2)
    .map((a) => `${a.asset} ${fmtSigned(a.change ?? 0, 2)}`);
  const { cash, positionsChange } = figures(portfolio);
  const parts = [...movers];
  if (portfolio.positions.length && Math.abs(positionsChange) >= 0.005) parts.push(`合约 ${fmtSigned(positionsChange, 2)}`);
  parts.push(`稳定币 ${Math.round((cash / total) * 100)}%`);
  return parts.join(" · ");
}

export type Sort = "value" | "change";

/** The assets the list shows, in `sort` order; small ones only when asked for. */
export function listed(assets: HeldAsset[], sort: Sort, showSmall: boolean): { shown: HeldAsset[]; small: HeldAsset[] } {
  const small = assets.filter((a) => (a.value ?? 0) < SMALL);
  const shown = (showSmall ? assets : assets.filter((a) => !small.includes(a))).slice();
  if (sort === "change") {
    shown.sort((a, b) => Math.abs(b.change ?? 0) - Math.abs(a.change ?? 0));
  }
  return { shown, small };
}

/** How far the mark is from liquidation, as a share of the mark; null without a liquidation price. */
export function liquidationDistance(position: HeldPosition): number | null {
  if (position.liquidation === null || !(position.mark > 0)) return null;
  return Math.abs(position.mark - position.liquidation) / position.mark;
}

/** Return on the margin behind a position, in percent, when the leverage is known. */
export function returnOnMargin(position: HeldPosition): number | null {
  const pnl = position.pnlUsd ?? (position.pnlAsset === "USDT" ? position.pnl : null);
  if (pnl === null || position.leverage === null || !(position.leverage > 0)) return null;
  const margin = Math.abs(position.exposure) / position.leverage;
  return margin > 0 ? (pnl / margin) * 100 : null;
}
