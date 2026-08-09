import { GripVertical } from "lucide-react"
import * as ResizablePrimitive from "react-resizable-panels"

import { cn } from "@/lib/utils"

function ResizablePanelGroup({ className, ...props }: React.ComponentProps<typeof ResizablePrimitive.Group>) {
  return <ResizablePrimitive.Group className={cn("flex size-full", className)} {...props} />
}

const ResizablePanel = ResizablePrimitive.Panel

function ResizableHandle({ className, withHandle = false, ...props }: React.ComponentProps<typeof ResizablePrimitive.Separator> & { withHandle?: boolean }) {
  return (
    <ResizablePrimitive.Separator
      className={cn("relative flex items-center justify-center bg-border focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring aria-[orientation=horizontal]:h-px aria-[orientation=horizontal]:w-full aria-[orientation=horizontal]:[&>div]:rotate-90 aria-[orientation=vertical]:h-full aria-[orientation=vertical]:w-px", className)}
      {...props}
    >
      {withHandle ? <div className="z-10 flex h-7 w-3 items-center justify-center rounded-sm border bg-border"><GripVertical className="size-3" /></div> : null}
    </ResizablePrimitive.Separator>
  )
}

export { ResizableHandle, ResizablePanel, ResizablePanelGroup }
