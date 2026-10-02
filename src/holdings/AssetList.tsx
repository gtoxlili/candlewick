import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { cn } from "cn";

import type { HeldAsset } from "@/lib/api";
import { fmtAmount, fmtPct, fmtPrice, fmtSigned, priceDecimals } from "@/lib/format";
import { seriesColor, type Sort } from "./summary";

/** "Binance Spot 0.5 · OKX Trading 0.26", or just the wallet when there is one. */
function whereHeld(asset: HeldAsset, severalExchanges: boolean, t: TFunction): string {
  const place = (h: HeldAsset["held"][number]) => {
    const wallet = t(`holdings.wallet.${h.wallet}`);
    return severalExchanges ? `${t(`common.provider.${h.exchange}`)} ${wallet}` : wallet;
  };
  if (asset.held.length === 1) return place(asset.held[0]);
  return asset.held.map((h) => `${place(h)} ${fmtAmount(h.amount)}`).join(" · ");
}

/** The up or down color for a number, none when it shows as flat or is unknown. */
export function trend(value: number | null, decimals = 2): string | undefined {
  if (value === null) return undefined;
  const sign = Math.sign(Number(value.toFixed(decimals)));
  return sign > 0 ? "text-up" : sign < 0 ? "text-down" : undefined;
}

const GRID = "grid-cols-[1.5fr_1fr_1fr_0.9fr_1.2fr]";
const GRID_COSTED = "grid-cols-[1.5fr_1fr_1fr_0.9fr_1.2fr_1.1fr]";

/**
 * Every asset, like the order book's ladder: compact rows of right-aligned
 * numbers, a bar behind the value for its share of the total (or, sorted by
 * the day's move, for the size of that move).
 */
export function AssetList(props: {
  assets: HeldAsset[];
  /** For the series colors: every asset, in value order. */
  all: HeldAsset[];
  total: number;
  sort: Sort;
  severalExchanges: boolean;
  /** How many small assets are folded away, and the toggle for them. */
  small: { count: number; value: number; shown: boolean; onToggle: () => void };
}) {
  const { t } = useTranslation();
  const { assets, sort } = props;
  const costed = assets.some((a) => a.cost !== null || a.pnl !== null);
  const grid = costed ? GRID_COSTED : GRID;
  const widestMove = Math.max(...assets.map((a) => Math.abs(a.change ?? 0)), 0) || 1;
  return (
    <div className="panel mx-4 flex min-h-0 flex-1 flex-col overflow-hidden text-xs tabular">
      <div className={cn("grid h-7 shrink-0 items-center gap-3 border-b px-3 text-2xs text-muted-foreground", grid)}>
        <span>{t("holdings.asset")}</span>
        <span className="text-right">{t("holdings.amount")}</span>
        <span className="text-right">{t("holdings.price")}</span>
        <span className="text-right">24h</span>
        <span className="text-right">{t("holdings.value")}</span>
        {costed && <span className="text-right">{t("holdings.costPnl")}</span>}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {assets.map((asset) => {
          const move = asset.change ?? 0;
          const bar =
            sort === "change"
              ? { width: (Math.abs(move) / widestMove) * 100, color: move >= 0 ? "var(--up)" : "var(--down)" }
              : { width: ((asset.value ?? 0) / props.total) * 100, color: seriesColor(asset, props.all) };
          const where = whereHeld(asset, props.severalExchanges, t);
          return (
            <div key={asset.asset} className={cn("relative grid h-9.5 items-center gap-3 px-3", grid)}>
              {/* Its share of the total, from the right like the book's depth. */}
              <div
                aria-hidden
                className="absolute inset-y-0.5 right-0 rounded-l-[3px] opacity-[0.09] transition-[width] duration-500 ease-out"
                style={{ width: `${Math.min(100, bar.width)}%`, background: bar.color }}
              />
              <div className="relative min-w-0">
                <div className="truncate font-medium text-foreground">{asset.asset}</div>
                <div className="truncate text-2xs text-muted-foreground" title={where}>
                  {where}
                </div>
              </div>
              <span className="relative text-right">{fmtAmount(asset.amount)}</span>
              <span className="relative text-right text-muted-foreground">
                {asset.price === null ? "—" : fmtPrice(asset.price, asset.stable ? 2 : priceDecimals(asset.price, null))}
              </span>
              <div className={cn("relative text-right", asset.stable && "text-faint")}>
                {asset.changePct === null ? (
                  "—"
                ) : (
                  <>
                    <div className={cn(!asset.stable && trend(asset.changePct))}>{fmtPct(asset.changePct)}</div>
                    {!asset.stable && asset.change !== null && (
                      <div className={cn("text-2xs opacity-80", trend(asset.change))}>{fmtSigned(asset.change, 2)}</div>
                    )}
                  </>
                )}
              </div>
              <div className="relative text-right font-medium">
                {asset.value === null ? "—" : fmtPrice(asset.value, 2)}
              </div>
              {costed && (
                <div className="relative text-right">
                  {asset.cost === null ? (
                    <span className="text-faint">—</span>
                  ) : (
                    <>
                      <div className="text-muted-foreground">{fmtPrice(asset.cost, priceDecimals(asset.cost, null))}</div>
                      {asset.pnl !== null && (
                        <div className={cn("text-2xs", trend(asset.pnl))}>{fmtSigned(asset.pnl, 2)}</div>
                      )}
                    </>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {props.small.count > 0 && (
          <button
            type="button"
            onClick={props.small.onToggle}
            className="flex h-8 w-full items-center justify-between px-3 text-2xs text-muted-foreground hover:text-foreground"
          >
            <span>
              {props.small.shown
                ? t("holdings.smallHide", { count: props.small.count })
                : t("holdings.smallMore", { count: props.small.count })}
            </span>
            <span>{t("holdings.smallTotal", { value: fmtPrice(props.small.value, 2) })}</span>
          </button>
        )}
      </div>
    </div>
  );
}
