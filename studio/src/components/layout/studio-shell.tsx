import { Waypoints } from "lucide-react"

import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

interface StudioShellProps {
  navigation: React.ReactNode
  timeline: React.ReactNode
  graph: React.ReactNode
  answer: React.ReactNode
  mobileTab: string
  onMobileTabChange: (value: string) => void
}

export function StudioShell(props: StudioShellProps): React.ReactElement {
  return (
    <main className="flex h-screen min-h-[42rem] flex-col overflow-hidden bg-background">
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-4">
        <div className="flex items-center gap-2"><span className="flex size-7 items-center justify-center rounded-md bg-primary/15 text-primary"><Waypoints className="size-4" /></span><div><h1 className="text-sm font-semibold tracking-tight">GraphLoom Studio</h1><p className="text-[10px] text-muted-foreground">Query-visible graph observatory</p></div></div>
        <div className="font-mono text-[10px] text-muted-foreground">trusted local MVP</div>
      </header>

      <div className="hidden min-h-0 flex-1 lg:block">
        <ResizablePanelGroup orientation="horizontal">
          <ResizablePanel defaultSize="22%" minSize="16%" maxSize="32%"><div className="size-full overflow-hidden border-r p-3">{props.navigation}</div></ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize="45%" minSize="30%">
            <ResizablePanelGroup orientation="vertical">
              <ResizablePanel defaultSize="62%" minSize="32%">{props.timeline}</ResizablePanel>
              <ResizableHandle withHandle />
              <ResizablePanel defaultSize="38%" minSize="22%">{props.answer}</ResizablePanel>
            </ResizablePanelGroup>
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize="33%" minSize="24%"><div className="size-full overflow-hidden border-l">{props.graph}</div></ResizablePanel>
        </ResizablePanelGroup>
      </div>

      <div className="flex min-h-0 flex-1 flex-col lg:hidden">
        <Tabs value={props.mobileTab} onValueChange={props.onMobileTabChange} className="flex min-h-0 flex-1 flex-col p-2">
          <TabsList className="grid w-full grid-cols-3"><TabsTrigger value="runs">Query / Runs</TabsTrigger><TabsTrigger value="timeline">Timeline</TabsTrigger><TabsTrigger value="graph">Graph</TabsTrigger></TabsList>
          <TabsContent value="runs" className="min-h-0 flex-1 overflow-hidden p-2">{props.navigation}</TabsContent>
          <TabsContent value="timeline" className="min-h-0 flex-1 overflow-hidden">{props.timeline}</TabsContent>
          <TabsContent value="graph" className="min-h-0 flex-1 overflow-hidden">{props.graph}</TabsContent>
        </Tabs>
        <div className="h-[38vh] min-h-64 border-t">{props.answer}</div>
      </div>
    </main>
  )
}
