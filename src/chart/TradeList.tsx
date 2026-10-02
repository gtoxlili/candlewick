import { useTranslation } from "react-i18next";
import { cn } from "cn";

import { Spinner } from "@/components/ui/spinner";
import type { Trade } from "@/lib/api";
import { fmtClock, fmtPrice, fmtQty } from "@/lib/format";

export function TradeList(props: { trades: Trade[] | null; decimals: number; base: string; quote: string }) {
  const { t, i18n } = useTranslation();
  return (
    <div className="flex min-h-0 flex-1 flex-col text-xs tabular">
      <div className="grid h-6 shrink-0 grid-cols-[1.1fr_1fr_0.9fr] items-center px-3 text-2xs text-muted-foreground">
        <span>{props.quote ? t("chart.priceIn", { unit: props.quote }) : t("chart.price")}</span>
        <span className="text-right">{props.quote ? t("chart.sizeIn", { unit: props.base }) : t("chart.size")}</span>
        <span className="text-right">{t("chart.time")}</span>
      </div>
      {props.trades === null ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <Spinner aria-label={t("loading")} />
        </div>
      ) : props.trades.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">{t("chart.noTrades")}</div>
      ) : (
        <div className="min-h-0 flex-1 overflow-hidden [mask-image:linear-gradient(to_top,transparent,black_2.5rem)]">
          {props.trades.map((trade) => (
            <div
              key={trade.id}
              className="grid h-5 grid-cols-[1.1fr_1fr_0.9fr] items-center px-3 animate-in fade-in slide-in-from-top-1 duration-300"
            >
              <span className={cn(trade.sell ? "text-down" : "text-up")}>
                {fmtPrice(trade.price, props.decimals)}
              </span>
              <span className="text-right">{fmtQty(trade.qty, i18n.language)}</span>
              <span className="text-right text-muted-foreground">{fmtClock(trade.time)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
