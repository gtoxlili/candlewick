import { cn } from "cn";

import { Badge } from "@/components/ui/badge";
import { direction, fmtPct } from "@/lib/format";

/** "+1.23% · +1,024.50 24h": a change in percent and amount, in the trend colors. */
export function ChangeBadge(props: {
  pct: number;
  /** The change as an amount, already signed and formatted. */
  amount: string;
  /** What the change covers, e.g. "24h". */
  span: string;
  className?: string;
}) {
  const sign = direction(props.pct);
  return (
    <Badge
      className={cn(
        "tabular",
        sign > 0 && "bg-up/12 text-up",
        sign < 0 && "bg-down/12 text-down",
        sign === 0 && "bg-fill-strong text-muted-foreground",
        props.className,
      )}
    >
      {fmtPct(props.pct)} · {props.amount}
      <span className="opacity-70">{props.span}</span>
    </Badge>
  );
}
