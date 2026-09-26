import * as React from "react"
import { cn } from "cn"
import { Switch as SwitchPrimitive } from "radix-ui"

// The system's own switch. macOS: an accent track when on, a white thumb in
// both appearances. Windows 11: an outlined track with a small knob when off,
// filled with the accent when on; the knob grows under the pointer and
// stretches while pressed.
const MACOS = {
  root: "border-transparent p-px data-[size=default]:h-5.5 data-[size=default]:w-9.5 data-[size=sm]:h-4 data-[size=sm]:w-6.5 data-checked:bg-primary data-unchecked:bg-black/10 dark:data-unchecked:bg-white/15",
  thumb:
    "bg-white shadow-[0_1px_2px_rgb(0_0_0/0.25),0_0_0_0.5px_rgb(0_0_0/0.06)] transition-transform duration-150 group-data-[size=default]/switch:size-5 group-data-[size=sm]/switch:size-3.5 group-data-[size=default]/switch:data-checked:translate-x-4 group-data-[size=sm]/switch:data-checked:translate-x-2.5 data-unchecked:translate-x-0",
}
const WINDOWS = {
  root: "data-[size=default]:h-5 data-[size=default]:w-10 data-[size=sm]:h-4 data-[size=sm]:w-8 data-checked:border-transparent data-checked:bg-primary data-checked:hover:bg-primary/90 data-unchecked:border-black/45 data-unchecked:bg-black/[0.024] data-unchecked:hover:bg-black/[0.058] dark:data-unchecked:border-white/55 dark:data-unchecked:bg-black/10 dark:data-unchecked:hover:bg-white/[0.042]",
  thumb:
    "transition-[translate,scale,background-color] duration-150 ease-out group-hover/switch:scale-[1.17] group-active/switch:scale-x-[1.42] group-active/switch:scale-y-[1.17] group-data-[size=default]/switch:ml-[3px] group-data-[size=default]/switch:size-3 group-data-[size=sm]/switch:ml-[2px] group-data-[size=sm]/switch:size-2.5 group-data-[size=default]/switch:data-checked:translate-x-5 group-data-[size=sm]/switch:data-checked:translate-x-4 data-checked:bg-primary-foreground data-unchecked:translate-x-0 data-unchecked:bg-black/45 dark:data-unchecked:bg-white/55",
}
const LOOK = __WINDOWS__ ? WINDOWS : MACOS

function Switch({
  className,
  size = "default",
  ...props
}: React.ComponentProps<typeof SwitchPrimitive.Root> & {
  size?: "sm" | "default"
}) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      data-size={size}
      className={cn(
        "peer group/switch relative inline-flex shrink-0 items-center rounded-full border transition-colors outline-none after:absolute after:-inset-x-3 after:-inset-y-2 focus-visible:ring-3 focus-visible:ring-ring data-disabled:cursor-not-allowed data-disabled:opacity-50",
        LOOK.root,
        className
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={cn("pointer-events-none block rounded-full", LOOK.thumb)}
      />
    </SwitchPrimitive.Root>
  )
}

export { Switch }
