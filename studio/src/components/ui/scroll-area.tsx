import { ScrollArea as ScrollAreaPrimitive } from "radix-ui"
import type { ComponentProps, Ref } from "react"

import { cn } from "@/lib/utils"

interface ScrollAreaProps extends ComponentProps<typeof ScrollAreaPrimitive.Root> {
  viewportRef?: Ref<HTMLDivElement>
}

function ScrollArea({ className, children, viewportRef, ...props }: ScrollAreaProps) {
  return (
    <ScrollAreaPrimitive.Root className={cn("relative overflow-hidden", className)} {...props}>
      <ScrollAreaPrimitive.Viewport ref={viewportRef} className="size-full rounded-[inherit]">
        {children}
      </ScrollAreaPrimitive.Viewport>
      <ScrollAreaPrimitive.Scrollbar
        orientation="vertical"
        className="flex w-2.5 touch-none select-none p-px"
      >
        <ScrollAreaPrimitive.Thumb className="relative flex-1 rounded-full bg-border" />
      </ScrollAreaPrimitive.Scrollbar>
      <ScrollAreaPrimitive.Corner />
    </ScrollAreaPrimitive.Root>
  )
}

export { ScrollArea }
