import { LoaderCircle } from "lucide-react";
import { cn } from "cn";

import type { Trade } from "@/lib/api";
import { fmtClock, fmtPrice, fmtQty } from "@/lib/format";

export function TradeList(props: { trades: Trade[] | null; decimals: number; base: string; quote: string }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col text-xs tabular">
      <div className="grid h-6 shrink-0 grid-cols-[1.1fr_1fr_0.9fr] items-center px-3 text-[11px] text-muted-foreground">
        <span>{props.quote ? `价格(${props.quote})` : "价格"}</span>
        <span className="text-right">{props.quote ? `数量(${props.base})` : "数量"}</span>
        <span className="text-right">时间</span>
      </div>
      {props.trades === null ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <LoaderCircle className="size-4 animate-spin" />
        </div>
      ) : props.trades.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">暂无成交</div>
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
              <span className="text-right">{fmtQty(trade.qty)}</span>
              <span className="text-right text-muted-foreground">{fmtClock(trade.time)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
