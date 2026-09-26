import { ArrowDown, ArrowUp } from "lucide-react";
import { cn } from "cn";

import { fmtPrice, fmtQty } from "@/lib/format";
import type { Book, Level } from "./market";

interface Row extends Level {
  total: number;
}

/** Running total from the best level outwards. */
function withTotals(levels: Level[]): Row[] {
  let total = 0;
  return levels.map((level) => ({ ...level, total: (total += level.qty) }));
}

/** Share of resting size on the bid side across the visible depth, 0–100. */
export function bidShare(book: Book | null): number | null {
  if (!book) return null;
  const bids = book.bids.reduce((sum, l) => sum + l.qty, 0);
  const asks = book.asks.reduce((sum, l) => sum + l.qty, 0);
  return bids + asks > 0 ? (bids / (bids + asks)) * 100 : null;
}

export function OrderBook(props: {
  book: Book | null;
  last: number | null;
  /** How the last price moved at the previous tick. */
  lastDirection: 1 | -1 | 0;
  decimals: number;
  base: string;
  quote: string;
}) {
  const { book, decimals } = props;
  const asks = withTotals(book?.asks ?? []);
  const bids = withTotals(book?.bids ?? []);
  // One scale for both sides so their depth bars compare.
  const deepest = Math.max(asks.at(-1)?.total ?? 0, bids.at(-1)?.total ?? 0) || 1;
  const spread = book?.asks[0] && book.bids[0] ? book.asks[0].price - book.bids[0].price : null;
  const share = bidShare(book);

  return (
    <div className="flex min-h-0 flex-1 flex-col text-xs tabular">
      {share !== null && (
        <div className="shrink-0 px-3 pb-2">
          <div className="mb-1 flex justify-between text-[11px]">
            <span className="text-up">买 {share.toFixed(0)}%</span>
            <span className="text-down">{(100 - share).toFixed(0)}% 卖</span>
          </div>
          <div className="flex h-1 gap-0.5 overflow-hidden rounded-full">
            <div className="rounded-full bg-up transition-[width] duration-700 ease-out" style={{ width: `${share}%` }} />
            <div className="flex-1 rounded-full bg-down" />
          </div>
        </div>
      )}
      <div className="grid h-6 shrink-0 grid-cols-[1.1fr_1fr_1fr] items-center px-3 text-[11px] text-muted-foreground">
        <span>{props.quote ? `价格(${props.quote})` : "价格"}</span>
        <span className="text-right">{props.quote ? `数量(${props.base})` : "数量"}</span>
        <span className="text-right">累计</span>
      </div>
      {/* Best ask sits at the bottom, next to the price; far levels clip at the top. */}
      <div className="flex min-h-0 flex-1 flex-col-reverse overflow-hidden [mask-image:linear-gradient(to_bottom,transparent,black_2.5rem)]">
        {asks.map((row) => (
          <BookRow key={row.price} row={row} side="ask" decimals={decimals} deepest={deepest} />
        ))}
      </div>
      <div className="flex h-10 shrink-0 items-center gap-1 px-3">
        <span
          className={cn(
            "text-base font-semibold tracking-tight transition-colors duration-500",
            props.lastDirection > 0 && "text-up",
            props.lastDirection < 0 && "text-down",
          )}
        >
          {props.last === null ? "—" : fmtPrice(props.last, decimals)}
        </span>
        {props.lastDirection > 0 && <ArrowUp className="size-3.5 text-up" strokeWidth={2.5} />}
        {props.lastDirection < 0 && <ArrowDown className="size-3.5 text-down" strokeWidth={2.5} />}
        {spread !== null && (
          <span className="ml-auto rounded-full bg-black/5 px-2 py-0.5 text-[11px] text-muted-foreground dark:bg-white/8">
            价差 {fmtPrice(spread, decimals)}
          </span>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-hidden [mask-image:linear-gradient(to_top,transparent,black_2.5rem)]">
        {bids.map((row) => (
          <BookRow key={row.price} row={row} side="bid" decimals={decimals} deepest={deepest} />
        ))}
      </div>
    </div>
  );
}

function BookRow(props: { row: Row; side: "bid" | "ask"; decimals: number; deepest: number }) {
  const { row, side } = props;
  return (
    <div className="relative grid h-5 shrink-0 grid-cols-[1.1fr_1fr_1fr] items-center px-3">
      <div
        className={cn(
          "absolute inset-y-0.5 right-0 rounded-l-[3px] transition-[width] duration-500 ease-out",
          side === "bid" ? "bg-up/10" : "bg-down/10",
        )}
        style={{ width: `${(row.total / props.deepest) * 100}%` }}
      />
      <span className={cn("relative", side === "bid" ? "text-up" : "text-down")}>
        {fmtPrice(row.price, props.decimals)}
      </span>
      <span className="relative text-right">{fmtQty(row.qty)}</span>
      <span className="relative text-right text-muted-foreground">{fmtQty(row.total)}</span>
    </div>
  );
}
