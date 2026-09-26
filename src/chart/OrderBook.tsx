import { ArrowDown, ArrowUp, LoaderCircle } from "lucide-react";
import { cn } from "cn";

import { Picker } from "@/components/Picker";
import type { Book, Level } from "@/lib/api";
import { fmtPrice, fmtQty } from "@/lib/format";

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
  /** Price steps the book can be grouped by, finest first. */
  steps: number[];
  /** Which of them `book` is grouped by. */
  step: number;
  onStep: (step: number) => void;
}) {
  const { book, decimals } = props;
  // Grouped prices need no more decimals than their step.
  const stepSize = props.steps.at(props.step);
  const rowDecimals = stepSize === undefined ? decimals : Math.min(decimals, stepDecimals(stepSize));
  const asks = withTotals(book?.asks ?? []);
  const bids = withTotals(book?.bids ?? []);
  // One scale for both sides so their depth bars compare.
  const deepest = Math.max(asks.at(-1)?.total ?? 0, bids.at(-1)?.total ?? 0) || 1;
  const spread = book?.asks[0] && book.bids[0] ? book.asks[0].price - book.bids[0].price : null;
  const share = bidShare(book);
  // Closed markets, halted stocks and indexes have no resting orders.
  const empty = book !== null && asks.length === 0 && bids.length === 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col text-xs tabular">
      {share !== null && (
        <div className="shrink-0 px-3 pb-2">
          <div className="mb-1 flex justify-between text-2xs">
            <span className="text-up">买 {share.toFixed(0)}%</span>
            <span className="text-down">{(100 - share).toFixed(0)}% 卖</span>
          </div>
          <div className="flex h-1 gap-0.5 overflow-hidden rounded-full">
            <div className="rounded-full bg-up transition-[width] duration-700 ease-out" style={{ width: `${share}%` }} />
            <div className="flex-1 rounded-full bg-down" />
          </div>
        </div>
      )}
      <div className="grid h-6 shrink-0 grid-cols-[1.1fr_1fr_1fr] items-center px-3 text-2xs text-muted-foreground">
        <span>{props.quote ? `价格(${props.quote})` : "价格"}</span>
        <span className="text-right">{props.quote ? `数量(${props.base})` : "数量"}</span>
        <span className="text-right">累计</span>
      </div>
      {book === null ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <LoaderCircle className="size-4 animate-spin" />
        </div>
      ) : empty ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">暂无挂单</div>
      ) : (
        <>
          {/* Best ask sits at the bottom, next to the price; far levels clip at the top. */}
          <div className="flex min-h-0 flex-1 flex-col-reverse overflow-hidden [mask-image:linear-gradient(to_bottom,transparent,black_2.5rem)]">
            {asks.map((row) => (
              <BookRow key={row.price} row={row} side="ask" decimals={rowDecimals} deepest={deepest} />
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
            {props.steps.length > 1 ? (
              <StepPicker steps={props.steps} step={props.step} onStep={props.onStep} />
            ) : (
              spread !== null && (
                <span className="ml-auto rounded-full bg-black/5 px-2 py-0.5 text-2xs text-muted-foreground dark:bg-white/8">
                  价差 {fmtPrice(spread, decimals)}
                </span>
              )
            )}
          </div>
          <div className="min-h-0 flex-1 overflow-hidden [mask-image:linear-gradient(to_top,transparent,black_2.5rem)]">
            {bids.map((row) => (
              <BookRow key={row.price} row={row} side="bid" decimals={rowDecimals} deepest={deepest} />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

/** The step the book is grouped by, opening the native pop-up menu of the others. */
function StepPicker(props: { steps: number[]; step: number; onStep: (step: number) => void }) {
  return (
    <Picker
      label="合并深度"
      value={String(props.step)}
      options={props.steps.map((step, i) => ({ value: String(i), label: fmtStep(step) }))}
      onChange={(value) => props.onStep(Number(value))}
      className="ml-auto flex items-center gap-0.5 rounded-full bg-black/5 py-0.5 pr-1.5 pl-2 text-2xs text-muted-foreground hover:bg-black/8 dark:bg-white/8 dark:hover:bg-white/12"
    >
      {fmtStep(props.steps[props.step])}
    </Picker>
  );
}

/** Decimals that show a step exactly: 0.001 → 3, 10 → 0. */
function stepDecimals(step: number): number {
  return Math.max(0, -Math.floor(Math.log10(step) + 1e-9));
}

function fmtStep(step: number): string {
  return fmtPrice(step, stepDecimals(step));
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
