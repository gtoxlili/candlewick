import { cn } from "cn";

import type { Share } from "./summary";

/**
 * The wick: one thin bar that shows how the total splits, largest holding
 * first, cash at the end, with its legend under it. Lit at the end while
 * the numbers are live.
 */
export function Allocation(props: { shares: Share[]; live: boolean }) {
  const { shares } = props;
  return (
    <div className="px-5">
      <div className="flex h-1.5 items-stretch gap-px">
        {shares.length === 0 ? (
          <div className="flex-1 rounded-full bg-fill-strong" />
        ) : (
          shares.map((share, index) => (
            <div
              key={share.label}
              title={`${share.label} ${share.pct.toFixed(1)}%`}
              className={cn(
                "min-w-0.5 transition-[flex-basis] duration-700 ease-out",
                index === 0 && "rounded-l-full",
                index === shares.length - 1 && "rounded-r-full",
              )}
              style={{ flexBasis: `${share.pct}%`, background: share.color }}
            />
          ))
        )}
        <span
          aria-hidden
          className={cn(
            "-my-0.5 -ml-px size-2.5 shrink-0 rounded-full",
            props.live ? "bg-live animate-[wick-pulse_2.4s_ease-out_infinite]" : "bg-fill-strong",
          )}
        />
      </div>
      <ul className="mt-2 flex h-4 flex-wrap gap-x-3.5 overflow-hidden text-2xs text-muted-foreground tabular">
        {shares.map((share) => (
          <li key={share.label} className="flex items-center gap-1.5">
            <span className="size-1.5 rounded-full" style={{ background: share.color }} />
            <span className="text-foreground">{share.label}</span>
            {share.pct.toFixed(share.pct < 10 ? 1 : 0)}%
          </li>
        ))}
      </ul>
    </div>
  );
}
