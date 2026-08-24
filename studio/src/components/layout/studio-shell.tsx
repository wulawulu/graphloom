import { useState } from "react"
import { ChevronLeft, ChevronRight, Waypoints } from "lucide-react"
import { useTranslation } from "react-i18next"

import { LanguageSelector } from "@/components/layout/language-selector"
import { Button } from "@/components/ui/button"
import { ResizableHandle, ResizablePanel, ResizablePanelGroup, usePanelRef } from "@/components/ui/resizable"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"

import { useDesktopLayout } from "./use-desktop-layout"

interface StudioShellProps {
  queryWorkspace: React.ReactNode
  graph: React.ReactNode
  mobileTab: string
  onMobileTabChange: (value: string) => void
}

export function StudioShell(props: StudioShellProps): React.ReactElement {
  const { t } = useTranslation()
  const desktop = useDesktopLayout()
  const queryPanelRef = usePanelRef()
  const [queryCollapsed, setQueryCollapsed] = useState(false)
  return (
    <main className="flex h-screen min-h-[42rem] flex-col overflow-hidden bg-background">
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-4">
        <div className="flex min-w-0 items-center gap-2"><span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary"><Waypoints className="size-4" /></span><div className="min-w-0"><h1 className="truncate text-sm font-semibold tracking-tight">{t("navigation.studioTitle")}</h1><p className="truncate text-[10px] text-muted-foreground">{t("navigation.tagline")}</p></div></div>
        <div className="flex shrink-0 items-center gap-2"><div className="hidden font-mono text-[10px] text-muted-foreground sm:block">{t("navigation.productDescription")}</div><LanguageSelector /></div>
      </header>

      {!desktop ? <Tabs value={props.mobileTab} onValueChange={props.onMobileTabChange} className="shrink-0 p-2 pb-0"><TabsList className="grid w-full grid-cols-2"><TabsTrigger value="query">{t("navigation.query")}</TabsTrigger><TabsTrigger value="graph">{t("navigation.graphDetail")}</TabsTrigger></TabsList></Tabs> : null}
      <div className="relative min-h-0 flex-1">
        <ResizablePanelGroup orientation="horizontal" className={desktop ? "" : "relative"}>
          <ResizablePanel
            panelRef={queryPanelRef}
            defaultSize="350px"
            minSize="320px"
            maxSize="440px"
            collapsedSize="44px"
            collapsible
            className={desktop ? "" : `!absolute !inset-0 !size-full ${props.mobileTab === "query" ? "!visible !pointer-events-auto !z-10" : "!invisible !pointer-events-none"}`}
            onResize={(size) => setQueryCollapsed(size.inPixels <= 48)}
          >
            <div className={`relative size-full overflow-hidden border-r ${!desktop && props.mobileTab !== "query" ? "hidden" : ""}`}>
              <div className={desktop && queryCollapsed ? "invisible size-full" : "size-full"}>{props.queryWorkspace}</div>
              {desktop ? <Button
                variant="ghost"
                size="icon"
                className={`absolute top-2 z-10 size-8 ${queryCollapsed ? "left-1.5" : "right-2"}`}
                aria-label={queryCollapsed ? t("query.workspace.expand") : t("query.workspace.collapse")}
                aria-expanded={!queryCollapsed}
                onClick={() => queryCollapsed ? queryPanelRef.current?.expand() : queryPanelRef.current?.collapse()}
              >
                {queryCollapsed ? <ChevronRight /> : <ChevronLeft />}
              </Button> : null}
            </div>
          </ResizablePanel>
          <ResizableHandle withHandle className={desktop ? "" : "hidden"} />
          <ResizablePanel minSize="520px" className={desktop ? "" : `!absolute !inset-0 !size-full ${props.mobileTab === "graph" ? "!visible !pointer-events-auto !z-10" : "!invisible !pointer-events-none"}`}><div className={`size-full overflow-hidden ${!desktop && props.mobileTab !== "graph" ? "hidden" : ""}`}>{props.graph}</div></ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </main>
  )
}
