import { useCallback, useEffect, useRef, useState } from "react"
import { ChevronLeft, ChevronRight, Database, TriangleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"

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
import type { StudioTranslationKey } from "@/i18n/types"
import type { GraphHighlight } from "@/lib/explainability"
import type { ExplainabilityRecordView } from "@/lib/semantic-timeline"

import { GraphInspector, type GraphDetail } from "./graph-detail"
import { CommunityList, EntityList, RelationshipList } from "./graph-lists"
import { NetworkPreview } from "./network-preview"

export type GraphViewMode = "overview" | "query-focus" | "explorer-focus"
export type GraphFocusKind = "final-context" | "focus-target"
export type GraphFocusIntent = GraphSubgraphRequest & { revision: number; focusKind?: GraphFocusKind }
export type GraphInspectIntent = { candidate: ExplainabilityRecordView; revision: number }

type GraphDetailIdentity =
  | { kind: "entity"; id: string; decision: ExplainabilityRecordView | null }
  | { kind: "relationship"; id: string; decision: ExplainabilityRecordView | null }
  | { kind: "community"; id: string; decision: null }

type DetailNavigationKind = "root" | "internal" | "back"

interface GraphExplorerProps {
  runId: string | null
  focusIntent: GraphFocusIntent | null
  onClearFocus: () => void
  emphasisIntent?: GraphEmphasisIntent | null
  onClearEmphasis?: () => void
  inspectIntent?: GraphInspectIntent | null
  navigationResetRevision?: number
}

interface ProjectionRequest {
  controller: AbortController
  kind: GraphViewMode
}

type ExplorerOrigin =
  | { kind: "overview"; runId: string | null }
  | { kind: "query-focus"; runId: string | null; request: GraphSubgraphRequest; focusKind: GraphFocusKind }

interface CommittedQueryFocus {
  request: GraphSubgraphRequest
  focusKind: GraphFocusKind
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

async function fetchGraphDetail(identity: GraphDetailIdentity, signal: AbortSignal): Promise<GraphDetail> {
  if (identity.kind === "entity") return { kind: "entity", value: await getEntity(identity.id, signal) }
  if (identity.kind === "relationship") return { kind: "relationship", value: await getRelationship(identity.id, signal) }
  const [value, report] = await Promise.all([
    getCommunity(identity.id, signal),
    getCommunityReport(identity.id, signal).catch((reason: unknown) => {
      if (isAbort(reason)) throw reason
      if (reason instanceof ApiError && reason.status === 404) return null
      throw reason
    }),
  ])
  return { kind: "community", value, report }
}

function detailIdentity(detail: GraphDetail, decision: ExplainabilityRecordView | null): GraphDetailIdentity {
  if (detail.kind === "entity") return { kind: "entity", id: detail.value.id, decision }
  if (detail.kind === "relationship") return { kind: "relationship", id: detail.value.id, decision }
  return { kind: "community", id: detail.value.id, decision: null }
}

export function GraphExplorer({ emphasisIntent = null, focusIntent, inspectIntent = null, navigationResetRevision = 0, onClearEmphasis = noop, onClearFocus, runId }: GraphExplorerProps): React.ReactElement {
  const { t } = useTranslation()
  const desktop = useDesktopLayout()
  const [summary, setSummary] = useState<GraphSummary | null>(null)
  const [summaryError, setSummaryError] = useState(false)
  const [projection, setProjection] = useState<GraphProjection | null>(null)
  const [mode, setMode] = useState<GraphViewMode>("overview")
  const [unavailable, setUnavailable] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<StudioTranslationKey | null>(null)
  const [inspectorTab, setInspectorTab] = useState("inspect")
  const [mobileView, setMobileView] = useState("graph")
  const [inspectorCollapsed, setInspectorCollapsed] = useState(!desktop)
  const [detail, setDetail] = useState<GraphDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState(false)
  const [decision, setDecision] = useState<ExplainabilityRecordView | null>(null)
  const [detailHistory, setDetailHistory] = useState<GraphDetailIdentity[]>([])
  const [explorerOrigin, setExplorerOrigin] = useState<ExplorerOrigin | null>(null)
  const [focusCore, setFocusCore] = useState<GraphHighlight | null>(null)
  const [focusKind, setFocusKind] = useState<GraphFocusKind>("focus-target")
  const projectionRef = useRef<GraphProjection | null>(null)
  const modeRef = useRef<GraphViewMode>("overview")
  const projectionRequest = useRef<ProjectionRequest | null>(null)
  const summaryRequest = useRef<AbortController | null>(null)
  const detailRequest = useRef<AbortController | null>(null)
  const initialFocusIntent = useRef(focusIntent)
  const lifecycleInitialized = useRef(false)
  const previousFocusIntent = useRef(focusIntent)
  const previousRunId = useRef(runId)
  const previousInspectRevision = useRef<number | null>(null)
  const inspectionRunId = useRef(runId)
  const queryFocusRequest = useRef<CommittedQueryFocus | null>(null)
  const previousNavigationResetRevision = useRef(navigationResetRevision)
  const candidateInspectionActive = useRef(false)
  const inspectorPanelRef = usePanelRef()

  const commitProjection = useCallback((value: GraphProjection, nextMode: GraphViewMode, core: GraphHighlight | null, nextFocusKind: GraphFocusKind): void => {
    onClearEmphasis()
    projectionRef.current = value
    modeRef.current = nextMode
    setProjection(value)
    setMode(nextMode)
    setFocusCore(core)
    setFocusKind(nextFocusKind)
    setUnavailable(false)
  }, [onClearEmphasis])

  const invalidateCommittedFocus = useCallback((): void => {
    if (modeRef.current === "overview") return
    modeRef.current = "overview"
    projectionRef.current = null
    setMode("overview")
    setProjection(null)
    setFocusCore(null)
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
        if (projectionRequest.current === request && !controller.signal.aborted) {
          queryFocusRequest.current = null
          commitProjection(value, "overview", null, "focus-target")
        }
      })
      .catch((reason: unknown) => {
        if (projectionRequest.current !== request || isAbort(reason)) return
        if (projectionRef.current === null) setUnavailable(true)
        else setError("graph.messages.couldNotReloadGraphOverview")
      })
      .finally(() => {
        if (projectionRequest.current === request) {
          projectionRequest.current = null
          setLoading(false)
        }
      })
  }, [commitProjection])

  const loadFocus = useCallback((subgraphRequest: GraphSubgraphRequest, nextMode: Exclude<GraphViewMode, "overview">, nextFocusKind: GraphFocusKind = "focus-target"): void => {
    projectionRequest.current?.controller.abort()
    const controller = new AbortController()
    const request: ProjectionRequest = { controller, kind: nextMode }
    projectionRequest.current = request
    setLoading(true)
    setError(null)
    void getGraphSubgraph(subgraphRequest, controller.signal)
      .then((value) => {
        if (projectionRequest.current === request && !controller.signal.aborted) {
          if (nextMode === "query-focus") queryFocusRequest.current = { request: subgraphRequest, focusKind: nextFocusKind }
          commitProjection(value, nextMode, {
            entityIds: [...subgraphRequest.entity_ids],
            relationshipIds: [...subgraphRequest.relationship_ids],
          }, nextFocusKind)
        }
      })
      .catch((reason: unknown) => {
        if (projectionRequest.current === request && !isAbort(reason)) setError("graph.messages.couldNotLoadFocusedGraph")
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
        loadFocus(subgraphRequest(focusIntent), "query-focus", focusIntent.focusKind ?? "focus-target")
      }
      return
    }

    const runChanged = previousRunId.current !== runId
    const previousIntent = previousFocusIntent.current
    previousRunId.current = runId

    if (runChanged) {
      setExplorerOrigin(null)
      queryFocusRequest.current = null
      const hadFocusedProjection = modeRef.current !== "overview"
      const hadPendingFocus = projectionRequest.current?.kind === "query-focus" || projectionRequest.current?.kind === "explorer-focus"
      invalidateCommittedFocus()
      if (focusIntent !== null && focusIntent.revision !== previousIntent?.revision) {
        previousFocusIntent.current = focusIntent
        loadFocus(subgraphRequest(focusIntent), "query-focus", focusIntent.focusKind ?? "focus-target")
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
        loadFocus(subgraphRequest(focusIntent), "query-focus", focusIntent.focusKind ?? "focus-target")
      }
    }
  }, [focusIntent, invalidateCommittedFocus, loadFocus, loadOverview, onClearFocus, runId])

  useEffect(() => {
    if (previousNavigationResetRevision.current === navigationResetRevision) return
    previousNavigationResetRevision.current = navigationResetRevision
    setExplorerOrigin(null)
    queryFocusRequest.current = null
    const pending = projectionRequest.current
    if (pending?.kind === "query-focus") {
      pending.controller.abort()
      projectionRequest.current = null
      setLoading(false)
      setError(null)
    }
  }, [navigationResetRevision])

  const loadExplorerFocus = useCallback((request: GraphSubgraphRequest): void => {
    if (modeRef.current !== "explorer-focus") {
      const queryFocus = queryFocusRequest.current
      setExplorerOrigin(queryFocus === null
        ? { kind: "overview", runId }
        : { kind: "query-focus", runId, request: queryFocus.request, focusKind: queryFocus.focusKind })
    }
    loadFocus(request, "explorer-focus")
  }, [loadFocus, runId])

  const backFromFocus = useCallback((): void => {
    if (modeRef.current === "explorer-focus") {
      const origin = explorerOrigin
      setExplorerOrigin(null)
      if (origin?.kind === "query-focus" && origin.runId === runId) loadFocus(origin.request, "query-focus", origin.focusKind)
      else loadOverview()
      return
    }
    if (focusIntent !== null) onClearFocus()
    else loadOverview()
  }, [explorerOrigin, focusIntent, loadFocus, loadOverview, onClearFocus, runId])

  const reloadGraphData = useCallback((): void => {
    loadSummary()
    loadOverview()
  }, [loadOverview, loadSummary])

  const beginDetailRequest = useCallback((): AbortController => {
    detailRequest.current?.abort()
    const controller = new AbortController()
    detailRequest.current = controller
    setDetailLoading(true)
    setDetailError(false)
    return controller
  }, [])

  const finishDetailRequest = useCallback((controller: AbortController): void => {
    if (detailRequest.current === controller) {
      detailRequest.current = null
      setDetailLoading(false)
    }
  }, [])

  const resetCandidateInspection = useCallback((): void => {
    previousInspectRevision.current = null
    if (!candidateInspectionActive.current) return
    candidateInspectionActive.current = false
    detailRequest.current?.abort()
    detailRequest.current = null
    setDetail(null)
    setDetailHistory([])
    setDecision(null)
    setDetailLoading(false)
    setDetailError(false)
  }, [])

  const openDetail = useCallback((identity: GraphDetailIdentity, navigation: DetailNavigationKind): void => {
    setInspectorTab("inspect")
    setMobileView("detail")
    if (desktop) inspectorPanelRef.current?.expand()
    if (navigation === "root") {
      if (identity.decision !== null) candidateInspectionActive.current = true
      setDetailHistory([])
    }
    const previous = detail === null ? null : detailIdentity(detail, decision)
    const controller = beginDetailRequest()
    void fetchGraphDetail(identity, controller.signal)
      .then((value) => {
        if (detailRequest.current !== controller || controller.signal.aborted) return
        if (navigation === "internal" && previous !== null) setDetailHistory((history) => [...history, previous])
        if (navigation === "back") setDetailHistory((history) => history.slice(0, -1))
        candidateInspectionActive.current = identity.decision !== null
        setDecision(identity.decision)
        setDetail(value)
      })
      .catch((reason: unknown) => {
        if (detailRequest.current !== controller || isAbort(reason)) return
        candidateInspectionActive.current = decision !== null
        setDetailError(true)
      })
      .finally(() => finishDetailRequest(controller))
  }, [beginDetailRequest, decision, desktop, detail, finishDetailRequest, inspectorPanelRef])

  const openEntity = useCallback((id: string) => openDetail({ kind: "entity", id, decision: null }, "root"), [openDetail])
  const openRelationship = useCallback((id: string) => openDetail({ kind: "relationship", id, decision: null }, "root"), [openDetail])
  const openCommunity = useCallback((id: string) => openDetail({ kind: "community", id, decision: null }, "root"), [openDetail])
  const navigateEntity = useCallback((id: string) => openDetail({ kind: "entity", id, decision: null }, "internal"), [openDetail])
  const navigateCommunity = useCallback((id: string) => openDetail({ kind: "community", id, decision: null }, "internal"), [openDetail])
  const backFromDetail = useCallback(() => {
    const target = detailHistory.at(-1)
    if (target !== undefined) openDetail(target, "back")
  }, [detailHistory, openDetail])

  useEffect(() => {
    const runChanged = inspectionRunId.current !== runId
    inspectionRunId.current = runId
    if (runChanged) {
      resetCandidateInspection()
      return
    }
    if (inspectIntent === null) {
      resetCandidateInspection()
      return
    }
    if (previousInspectRevision.current === inspectIntent.revision) return
    previousInspectRevision.current = inspectIntent.revision
    if (inspectIntent.candidate.recordType === "entity") openDetail({ kind: "entity", id: inspectIntent.candidate.stableId, decision: inspectIntent.candidate }, "root")
    if (inspectIntent.candidate.recordType === "relationship") openDetail({ kind: "relationship", id: inspectIntent.candidate.stableId, decision: inspectIntent.candidate }, "root")
  }, [inspectIntent, openDetail, resetCandidateInspection, runId])

  const closeDetail = (): void => {
    candidateInspectionActive.current = false
    detailRequest.current?.abort()
    detailRequest.current = null
    setDetail(null)
    setDetailHistory([])
    setDetailLoading(false)
    setDetailError(false)
    setDecision(null)
  }

  const graphCanvas = (
    <div className="flex size-full min-h-0 flex-col p-2">
            <header className="flex h-9 shrink-0 items-center justify-between px-1">
              <div className="flex items-center gap-2"><Database className="size-4 text-primary" /><h2 className="text-sm font-semibold">{t("explainability.labels.knowledgeGraph")}</h2></div>
              {projection !== null ? <Badge variant="success">{t("graph.status.ready")}</Badge> : <Badge variant={unavailable ? "destructive" : "outline"}>{t(unavailable ? "graph.status.unavailable" : "graph.status.loading")}</Badge>}
            </header>
            {projection === null && loading ? <div className="space-y-3 p-4"><Skeleton className="h-12" /><Skeleton className="h-80" /></div> : null}
            {projection === null && !loading && unavailable ? <div className="flex flex-1 flex-col items-center justify-center p-6 text-center"><TriangleAlert className="mb-3 size-8 text-warning" /><p className="text-sm font-medium">{t("graph.labels.graphDataUnavailable")}</p><p className="mt-1 text-xs text-muted-foreground">{t("graph.messages.runGraphLoomIndexFirst")}</p></div> : null}
            {projection === null && !loading && !unavailable && error !== null ? <div className="flex flex-1 flex-col items-center justify-center p-6 text-center"><TriangleAlert className="mb-3 size-8 text-warning" /><p className="text-sm font-medium">{t(error)}</p><p className="mt-1 text-xs text-muted-foreground">{t("graph.messages.theFocusedRecordsCouldNotBeLoaded")}</p><Button className="mt-4" size="sm" variant="outline" onClick={loadOverview}>{t("graph.actions.loadOverview")}</Button></div> : null}
            {projection !== null ? <NetworkPreview projection={projection} summary={summary} summaryError={summaryError} mode={mode} focusCore={focusCore} focusKind={focusKind} loading={loading} error={error} emphasisIntent={emphasisIntent} onClearEmphasis={onClearEmphasis} onEntity={openEntity} onRelationship={openRelationship} onBack={backFromFocus} backLabel={mode === "explorer-focus" && explorerOrigin?.kind === "query-focus" ? "graph.actions.backToQueryFocus" : "graph.actions.backToOverview"} onReload={reloadGraphData} /> : null}
    </div>
  )
  const inspector = (
    <Tabs value={inspectorTab} onValueChange={setInspectorTab} className="flex size-full min-h-0 flex-col">
      <TabsList className="m-2 grid grid-cols-4"><TabsTrigger value="inspect">{t("graph.actions.inspect")}</TabsTrigger><TabsTrigger value="entities">{t("graph.labels.entities")}</TabsTrigger><TabsTrigger value="relationships">{t("graph.labels.relations")}</TabsTrigger><TabsTrigger value="communities">{t("graph.labels.groups")}</TabsTrigger></TabsList>
      <TabsContent value="inspect" className="min-h-0 flex-1"><GraphInspector detail={detail} decision={decision} loading={detailLoading} error={detailError} canGoBack={detailHistory.length > 0} onBack={backFromDetail} onClear={closeDetail} onOpenEntity={navigateEntity} onOpenCommunity={navigateCommunity} onFocusEntity={(id) => loadExplorerFocus({ entity_ids: [id], relationship_ids: [], depth: 1, max_entities: 80, max_relationships: 160 })} onFocusRelationship={(id) => loadExplorerFocus({ entity_ids: [], relationship_ids: [id], depth: 1, max_entities: 80, max_relationships: 160 })} /></TabsContent>
      <TabsContent value="entities" className="min-h-0 flex-1"><EntityList onSelect={openEntity} /></TabsContent>
      <TabsContent value="relationships" className="min-h-0 flex-1"><RelationshipList onSelect={openRelationship} /></TabsContent>
      <TabsContent value="communities" className="min-h-0 flex-1"><CommunityList onSelect={openCommunity} /></TabsContent>
    </Tabs>
  )

  return (
    <section className="flex size-full min-h-0 flex-col" aria-label={t("graph.labels.graphExplorer")}>
      <Tabs value={mobileView} onValueChange={setMobileView} className={desktop ? "hidden" : "shrink-0 p-2 pb-0"}>
        <TabsList className="grid w-full grid-cols-2"><TabsTrigger value="graph">{t("explainability.labels.graph")}</TabsTrigger><TabsTrigger value="detail">{t("graph.labels.detail")}</TabsTrigger></TabsList>
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
            {desktop ? <Button variant="ghost" size="icon" className={`absolute top-2 z-20 size-8 ${inspectorCollapsed ? "left-1.5" : "right-2"}`} aria-label={t(inspectorCollapsed ? "graph.actions.expandGraphInspector" : "graph.actions.collapseGraphInspector")} aria-expanded={!inspectorCollapsed} onClick={() => inspectorCollapsed ? inspectorPanelRef.current?.expand() : inspectorPanelRef.current?.collapse()}>{inspectorCollapsed ? <ChevronLeft /> : <ChevronRight />}</Button> : null}
          </aside>
        </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </section>
  )
}
