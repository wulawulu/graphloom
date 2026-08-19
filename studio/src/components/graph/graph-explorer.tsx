import { useCallback, useEffect, useRef, useState } from "react"
import { ChevronLeft, ChevronRight, Database, TriangleAlert } from "lucide-react"

import {
  ApiError,
  getCommunity,
  getCommunityReport,
  getEntity,
  getGraphOverview,
  getGraphSubgraph,
  getGraphSummary,
  getRelationship,
} from "@/api/client"
import type { GraphProjection, GraphSubgraphRequest, GraphSummary } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ResizableHandle, ResizablePanel, ResizablePanelGroup, usePanelRef } from "@/components/ui/resizable"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useDesktopLayout } from "@/components/layout/use-desktop-layout"
import type { GraphEmphasisIntent } from "@/lib/citations"

import { GraphInspector, type GraphDetail } from "./graph-detail"
import { CommunityList, EntityList, RelationshipList } from "./graph-lists"
import { NetworkPreview } from "./network-preview"

export type GraphViewMode = "overview" | "query-focus" | "explorer-focus"
export type GraphFocusIntent = GraphSubgraphRequest & { revision: number }

interface GraphExplorerProps {
  runId: string | null
  focusIntent: GraphFocusIntent | null
  onClearFocus: () => void
  emphasisIntent?: GraphEmphasisIntent | null
  onClearEmphasis?: () => void
}

interface ProjectionRequest {
  controller: AbortController
  kind: GraphViewMode
}

const noop = (): void => undefined

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}

function subgraphRequest(intent: GraphFocusIntent): GraphSubgraphRequest {
  return {
    entity_ids: intent.entity_ids,
    relationship_ids: intent.relationship_ids,
    depth: intent.depth,
    max_entities: intent.max_entities,
    max_relationships: intent.max_relationships,
  }
}

export function GraphExplorer({ emphasisIntent = null, focusIntent, onClearEmphasis = noop, onClearFocus, runId }: GraphExplorerProps): React.ReactElement {
  const desktop = useDesktopLayout()
  const [summary, setSummary] = useState<GraphSummary | null>(null)
  const [summaryError, setSummaryError] = useState(false)
  const [projection, setProjection] = useState<GraphProjection | null>(null)
  const [mode, setMode] = useState<GraphViewMode>("overview")
  const [unavailable, setUnavailable] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [inspectorTab, setInspectorTab] = useState("inspect")
  const [mobileView, setMobileView] = useState("graph")
  const [inspectorCollapsed, setInspectorCollapsed] = useState(!desktop)
  const [detail, setDetail] = useState<GraphDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState(false)
  const projectionRef = useRef<GraphProjection | null>(null)
  const modeRef = useRef<GraphViewMode>("overview")
  const projectionRequest = useRef<ProjectionRequest | null>(null)
  const summaryRequest = useRef<AbortController | null>(null)
  const detailRequest = useRef<AbortController | null>(null)
  const initialFocusIntent = useRef(focusIntent)
  const lifecycleInitialized = useRef(false)
  const previousFocusIntent = useRef(focusIntent)
  const previousRunId = useRef(runId)
  const inspectorPanelRef = usePanelRef()

  const commitProjection = useCallback((value: GraphProjection, nextMode: GraphViewMode): void => {
    onClearEmphasis()
    projectionRef.current = value
    modeRef.current = nextMode
    setProjection(value)
    setMode(nextMode)
    setUnavailable(false)
  }, [onClearEmphasis])

  const invalidateCommittedFocus = useCallback((): void => {
    if (modeRef.current === "overview") return
    modeRef.current = "overview"
    projectionRef.current = null
    setMode("overview")
    setProjection(null)
  }, [])

  const loadOverview = useCallback((): void => {
    projectionRequest.current?.controller.abort()
    const controller = new AbortController()
    const request: ProjectionRequest = { controller, kind: "overview" }
    projectionRequest.current = request
    setLoading(true)
    setError(null)
    void getGraphOverview({}, controller.signal)
      .then((value) => {
        if (projectionRequest.current === request && !controller.signal.aborted) commitProjection(value, "overview")
      })
      .catch((reason: unknown) => {
        if (projectionRequest.current !== request || isAbort(reason)) return
        if (projectionRef.current === null) setUnavailable(true)
        else setError("Could not reload graph overview.")
      })
      .finally(() => {
        if (projectionRequest.current === request) {
          projectionRequest.current = null
          setLoading(false)
        }
      })
  }, [commitProjection])

  const loadFocus = useCallback((subgraphRequest: GraphSubgraphRequest, nextMode: Exclude<GraphViewMode, "overview">): void => {
    projectionRequest.current?.controller.abort()
    const controller = new AbortController()
    const request: ProjectionRequest = { controller, kind: nextMode }
    projectionRequest.current = request
    setLoading(true)
    setError(null)
    void getGraphSubgraph(subgraphRequest, controller.signal)
      .then((value) => {
        if (projectionRequest.current === request && !controller.signal.aborted) commitProjection(value, nextMode)
      })
      .catch((reason: unknown) => {
        if (projectionRequest.current === request && !isAbort(reason)) setError("Could not load focused graph.")
      })
      .finally(() => {
        if (projectionRequest.current === request) {
          projectionRequest.current = null
          setLoading(false)
        }
      })
  }, [commitProjection])

  const loadSummary = useCallback((): void => {
    summaryRequest.current?.abort()
    const controller = new AbortController()
    summaryRequest.current = controller
    void getGraphSummary(controller.signal)
      .then((value) => {
        if (summaryRequest.current === controller && !controller.signal.aborted) {
          setSummary(value)
          setSummaryError(false)
        }
      })
      .catch((reason: unknown) => {
        if (summaryRequest.current === controller && !isAbort(reason)) setSummaryError(true)
      })
      .finally(() => {
        if (summaryRequest.current === controller) summaryRequest.current = null
      })
  }, [])

  useEffect(() => {
    loadSummary()
    if (initialFocusIntent.current === null) loadOverview()
    return () => {
      summaryRequest.current?.abort()
      projectionRequest.current?.controller.abort()
      detailRequest.current?.abort()
    }
  }, [loadOverview, loadSummary])

  useEffect(() => {
    if (!lifecycleInitialized.current) {
      lifecycleInitialized.current = true
      if (focusIntent !== null) {
        loadFocus(subgraphRequest(focusIntent), "query-focus")
      }
      return
    }

    const runChanged = previousRunId.current !== runId
    const previousIntent = previousFocusIntent.current
    previousRunId.current = runId

    if (runChanged) {
      const hadFocusedProjection = modeRef.current !== "overview"
      const hadPendingFocus = projectionRequest.current?.kind === "query-focus" || projectionRequest.current?.kind === "explorer-focus"
      invalidateCommittedFocus()
      if (focusIntent !== null && focusIntent.revision !== previousIntent?.revision) {
        previousFocusIntent.current = focusIntent
        loadFocus(subgraphRequest(focusIntent), "query-focus")
      } else if (focusIntent !== null) {
        previousFocusIntent.current = null
        onClearFocus()
        loadOverview()
      } else {
        previousFocusIntent.current = null
        if (hadFocusedProjection || hadPendingFocus) loadOverview()
      }
    } else {
      previousFocusIntent.current = focusIntent
      if (focusIntent === null) {
        if (previousIntent !== null) loadOverview()
      } else {
        loadFocus(subgraphRequest(focusIntent), "query-focus")
      }
    }
  }, [focusIntent, invalidateCommittedFocus, loadFocus, loadOverview, onClearFocus, runId])

  const backToOverview = useCallback((): void => {
    if (focusIntent !== null) onClearFocus()
    else loadOverview()
  }, [focusIntent, loadOverview, onClearFocus])

  const reloadGraphData = useCallback((): void => {
    loadSummary()
    loadOverview()
  }, [loadOverview, loadSummary])

  const beginDetailRequest = useCallback((): AbortController => {
    detailRequest.current?.abort()
    const controller = new AbortController()
    detailRequest.current = controller
    setDetailLoading(true)
    setDetail(null)
    setDetailError(false)
    return controller
  }, [])

  const finishDetailRequest = useCallback((controller: AbortController): void => {
    if (detailRequest.current === controller) {
      detailRequest.current = null
      setDetailLoading(false)
    }
  }, [])

  const openEntity = useCallback((id: string) => {
    setInspectorTab("inspect")
    setMobileView("detail")
    if (desktop) inspectorPanelRef.current?.expand()
    const controller = beginDetailRequest()
    void getEntity(id, controller.signal)
      .then((value) => { if (!controller.signal.aborted) setDetail({ kind: "entity", value }) })
      .catch(() => { if (!controller.signal.aborted) setDetailError(true) })
      .finally(() => finishDetailRequest(controller))
  }, [beginDetailRequest, desktop, finishDetailRequest, inspectorPanelRef])

  const openRelationship = useCallback((id: string) => {
    setInspectorTab("inspect")
    setMobileView("detail")
    if (desktop) inspectorPanelRef.current?.expand()
    const controller = beginDetailRequest()
    void getRelationship(id, controller.signal)
      .then((value) => { if (!controller.signal.aborted) setDetail({ kind: "relationship", value }) })
      .catch(() => { if (!controller.signal.aborted) setDetailError(true) })
      .finally(() => finishDetailRequest(controller))
  }, [beginDetailRequest, desktop, finishDetailRequest, inspectorPanelRef])

  const openCommunity = useCallback((id: string) => {
    setInspectorTab("inspect")
    setMobileView("detail")
    if (desktop) inspectorPanelRef.current?.expand()
    const controller = beginDetailRequest()
    void Promise.all([
      getCommunity(id, controller.signal),
      getCommunityReport(id, controller.signal).catch((reason: unknown) => {
        if (isAbort(reason)) throw reason
        if (reason instanceof ApiError && reason.status === 404) return null
        throw reason
      }),
    ])
      .then(([value, report]) => { if (!controller.signal.aborted) setDetail({ kind: "community", value, report }) })
      .catch(() => { if (!controller.signal.aborted) setDetailError(true) })
      .finally(() => finishDetailRequest(controller))
  }, [beginDetailRequest, desktop, finishDetailRequest, inspectorPanelRef])

  const closeDetail = (): void => {
    detailRequest.current?.abort()
    detailRequest.current = null
    setDetail(null)
    setDetailLoading(false)
    setDetailError(false)
  }

  const graphCanvas = (
    <div className="flex size-full min-h-0 flex-col p-2">
            <header className="flex h-9 shrink-0 items-center justify-between px-1">
              <div className="flex items-center gap-2"><Database className="size-4 text-primary" /><h2 className="text-sm font-semibold">Knowledge Graph</h2></div>
              {projection !== null ? <Badge variant="success">ready</Badge> : <Badge variant={unavailable ? "destructive" : "outline"}>{unavailable ? "unavailable" : "loading"}</Badge>}
            </header>
            {projection === null && loading ? <div className="space-y-3 p-4"><Skeleton className="h-12" /><Skeleton className="h-80" /></div> : null}
            {projection === null && !loading && unavailable ? <div className="flex flex-1 flex-col items-center justify-center p-6 text-center"><TriangleAlert className="mb-3 size-8 text-warning" /><p className="text-sm font-medium">Graph data unavailable</p><p className="mt-1 text-xs text-muted-foreground">Run GraphLoom index first.</p></div> : null}
            {projection === null && !loading && !unavailable && error !== null ? <div className="flex flex-1 flex-col items-center justify-center p-6 text-center"><TriangleAlert className="mb-3 size-8 text-warning" /><p className="text-sm font-medium">{error}</p><p className="mt-1 text-xs text-muted-foreground">The focused records could not be loaded.</p><Button className="mt-4" size="sm" variant="outline" onClick={loadOverview}>Load overview</Button></div> : null}
            {projection !== null ? <NetworkPreview projection={projection} summary={summary} summaryError={summaryError} mode={mode} loading={loading} error={error} emphasisIntent={emphasisIntent} onClearEmphasis={onClearEmphasis} onEntity={openEntity} onRelationship={openRelationship} onBackOverview={backToOverview} onReload={reloadGraphData} /> : null}
    </div>
  )
  const inspector = (
    <Tabs value={inspectorTab} onValueChange={setInspectorTab} className="flex size-full min-h-0 flex-col">
      <TabsList className="m-2 grid grid-cols-4"><TabsTrigger value="inspect">Inspect</TabsTrigger><TabsTrigger value="entities">Entities</TabsTrigger><TabsTrigger value="relationships">Relations</TabsTrigger><TabsTrigger value="communities">Groups</TabsTrigger></TabsList>
      <TabsContent value="inspect" className="min-h-0 flex-1"><GraphInspector detail={detail} loading={detailLoading} error={detailError} onClear={closeDetail} onFocusEntity={(id) => loadFocus({ entity_ids: [id], relationship_ids: [], depth: 1, max_entities: 80, max_relationships: 160 }, "explorer-focus")} onFocusRelationship={(id) => loadFocus({ entity_ids: [], relationship_ids: [id], depth: 1, max_entities: 80, max_relationships: 160 }, "explorer-focus")} /></TabsContent>
      <TabsContent value="entities" className="min-h-0 flex-1"><EntityList onSelect={openEntity} /></TabsContent>
      <TabsContent value="relationships" className="min-h-0 flex-1"><RelationshipList onSelect={openRelationship} /></TabsContent>
      <TabsContent value="communities" className="min-h-0 flex-1"><CommunityList onSelect={openCommunity} /></TabsContent>
    </Tabs>
  )

  return (
    <section className="flex size-full min-h-0 flex-col" aria-label="Graph Explorer">
      <Tabs value={mobileView} onValueChange={setMobileView} className={desktop ? "hidden" : "shrink-0 p-2 pb-0"}>
        <TabsList className="grid w-full grid-cols-2"><TabsTrigger value="graph">Graph</TabsTrigger><TabsTrigger value="detail">Detail</TabsTrigger></TabsList>
      </Tabs>
      <div className="relative min-h-0 flex-1">
        <ResizablePanelGroup orientation="horizontal" className={desktop ? "" : "relative"}>
        <ResizablePanel
          minSize="480px"
          className={desktop ? "" : `!absolute !inset-0 !size-full ${mobileView === "graph" ? "!visible !pointer-events-auto !z-10" : "!invisible !pointer-events-none"}`}
        >{graphCanvas}
        </ResizablePanel>
        <ResizableHandle withHandle className={desktop ? "" : "hidden"} />
        <ResizablePanel
          panelRef={inspectorPanelRef}
          defaultSize="330px"
          minSize="300px"
          maxSize="420px"
          collapsedSize="44px"
          collapsible
          className={desktop ? "" : `!absolute !inset-0 !size-full ${mobileView === "detail" ? "!visible !pointer-events-auto !z-10" : "!invisible !pointer-events-none"}`}
          onResize={(size) => setInspectorCollapsed(size.inPixels <= 48)}
        >
          <aside className="relative size-full min-h-0 border-l bg-card/20">
            <div className={desktop && inspectorCollapsed ? "invisible size-full" : "size-full"}>{inspector}</div>
            {desktop ? <Button variant="ghost" size="icon" className={`absolute top-2 z-20 size-8 ${inspectorCollapsed ? "left-1.5" : "right-2"}`} aria-label={inspectorCollapsed ? "Expand graph inspector" : "Collapse graph inspector"} aria-expanded={!inspectorCollapsed} onClick={() => inspectorCollapsed ? inspectorPanelRef.current?.expand() : inspectorPanelRef.current?.collapse()}>{inspectorCollapsed ? <ChevronLeft /> : <ChevronRight />}</Button> : null}
          </aside>
        </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </section>
  )
}
