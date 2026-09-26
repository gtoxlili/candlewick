import type { ReactNode } from "react";
import { ChevronDown, ChevronsUpDown } from "lucide-react";
import { cn } from "cn";
import { Select as SelectPrimitive } from "radix-ui";

import { Select, SelectContent, SelectGroup, SelectItem, SelectValue } from "@/components/ui/select";

/**
 * Picks one of `options` from a menu that opens on `children`, the current
 * value as the page shows it. macOS gets the system's pop-up menu (a hidden
 * `<select>` laid over the content). WebView2's `<select>` list is
 * Chromium's own, so Windows gets shadcn's Select, styled as Windows 11's
 * combo box drop-down (index.css), behind a trigger that keeps the page's look.
 */
export function Picker(props: {
  /** What is being picked, for screen readers. */
  label: string;
  value: string;
  options: readonly { value: string; label: string }[];
  onChange: (value: string) => void;
  /** The trigger's look; it lays out the value and the chevron after it. */
  className?: string;
  children: ReactNode;
}) {
  if (__WINDOWS__) {
    return (
      <Select value={props.value} onValueChange={props.onChange}>
        <SelectPrimitive.Trigger
          aria-label={props.label}
          className={cn("outline-none focus-visible:ring-2 focus-visible:ring-ring", props.className)}
        >
          <SelectValue>{props.children}</SelectValue>
          <ChevronDown className="size-3 text-muted-foreground" />
        </SelectPrimitive.Trigger>
        <SelectContent>
          <SelectGroup>
            {props.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
    );
  }
  return (
    <label className={cn("relative", props.className)}>
      {props.children}
      <ChevronsUpDown className="size-3 text-muted-foreground" />
      <select
        aria-label={props.label}
        className="absolute inset-0 appearance-none opacity-0"
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
      >
        {props.options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}
