import { cn } from "cn";

/**
 * Rounded pill tabs with a sliding selection, the chart window's control
 * style. Equal-width segments keep the indicator a pure transform.
 */
export function PillTabs<T extends string | number>(props: {
  value: T;
  options: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
  className?: string;
}) {
  const index = Math.max(0, props.options.findIndex((o) => o.value === props.value));
  const count = props.options.length;
  return (
    <div
      role="tablist"
      aria-label={props.label}
      className={cn(
        "pill relative grid shrink-0 p-0.5",
        props.className,
      )}
      style={{ gridTemplateColumns: `repeat(${count}, minmax(0, 1fr))` }}
    >
      <span
        aria-hidden
        className="absolute inset-y-0.5 left-0.5 rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/0.12),0_0_0_0.5px_rgb(0_0_0/0.04)] transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] dark:bg-white/16 dark:shadow-none"
        style={{ width: `calc((100% - 4px) / ${count})`, transform: `translateX(${index * 100}%)` }}
      />
      {props.options.map((option) => {
        const selected = option.value === props.value;
        return (
          <button
            key={String(option.value)}
            type="button"
            role="tab"
            aria-selected={selected}
            onClick={() => props.onChange(option.value)}
            className={cn(
              "relative h-6 px-2.5 text-xs whitespace-nowrap transition-colors duration-200",
              selected ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
