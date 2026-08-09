import { useCallback, useEffect, useRef, useState } from "react"
import { Database, TriangleAlert } from "lucide-react"

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
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

import { GraphDetailSheet, type GraphDetail } from "./graph-detail"
import { CommunityList, EntityList, RelationshipList } from "./graph-lists"
import { NetworkPreview } from "./network-preview"

export type GraphViewMode = "overview" | "focus"
export type GraphFocusIntent = GraphSubgraphRequest & { revision: number }

interface GraphExplorerProps { focusIntent: GraphFocusIntent | null }

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}

export function GraphExplorer({ focusIntent }: GraphExplorerProps): React.ReactElement {
  const [summary, setSummary] = useState<GraphSummary | null>(null)
  const [summaryError, setSummaryError] = useState(false)
  const [projection, setProjection] = useState<GraphProjection | null>(null)
  const [mode, setMode] = useState<GraphViewMode>("overview")
  const [unavailable, setUnavailable] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [tab, setTab] = useState("graph")
  const [detail, setDetail] = useState<GraphDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState(false)
  const projectionRef = useRef<GraphProjection | null>(null)
  const projectionRequest = useRef<AbortController | null>(null)
  const detailRequest = useRef<AbortController | null>(null)
  const initialFocusIntent = useRef(focusIntent)

  const commitProjection = useCallback((value: GraphProjection, nextMode: GraphViewMode): void => {
    projectionRef.current = value
    setProjection(value)
    setMode(nextMode)
    setUnavailable(false)
  }, [])

  const loadOverview = useCallback((): void => {
    projectionRequest.current?.abort()
    const controller = new AbortController()
    projectionRequest.current = controller
    setLoading(true)
    setError(null)
    void getGraphOverview({}, controller.signal)
      .then((value) => {
        if (projectionRequest.current === controller && !controller.signal.aborted) commitProjection(value, "overview")
      })
      .catch((reason: unknown) => {
        if (projectionRequest.current !== controller || isAbort(reason)) return
        if (projectionRef.current === null) setUnavailable(true)
        else setError("Could not reload graph overview.")
      })
      .finally(() => {
        if (projectionRequest.current === controller) {
          projectionRequest.current = null
          setLoading(false)
        }
      })
  }, [commitProjection])

  const loadFocus = useCallback((request: GraphSubgraphRequest): void => {
    projectionRequest.current?.abort()
    const controller = new AbortController()
    projectionRequest.current = controller
    setTab("graph")
    setLoading(true)
    setError(null)
    void getGraphSubgraph(request, controller.signal)
      .then((value) => {
        if (projectionRequest.current === controller && !controller.signal.aborted) commitProjection(value, "focus")
      })
      .catch((reason: unknown) => {
        if (projectionRequest.current === controller && !isAbort(reason)) setError("Could not load focused graph.")
      })
      .finally(() => {
        if (projectionRequest.current === controller) {
          projectionRequest.current = null
          setLoading(false)
        }
      })
  }, [commitProjection])

  useEffect(() => {
    const controller = new AbortController()
    void getGraphSummary(controller.signal)
      .then((value) => { if (!controller.signal.aborted) { setSummary(value); setSummaryError(false) } })
      .catch((reason: unknown) => { if (!isAbort(reason)) setSummaryError(true) })
    if (initialFocusIntent.current === null) loadOverview()
    return () => {
      controller.abort()
      projectionRequest.current?.abort()
      detailRequest.current?.abort()
    }
  }, [loadOverview])

  useEffect(() => {
    if (focusIntent === null) return
    loadFocus({
      entity_ids: focusIntent.entity_ids,
      relationship_ids: focusIntent.relationship_ids,
      depth: focusIntent.depth,
      max_entities: focusIntent.max_entities,
      max_relationships: focusIntent.max_relationships,
    })
  }, [focusIntent, loadFocus])

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
    const controller = beginDetailRequest()
    void getEntity(id, controller.signal)
      .then((value) => { if (!controller.signal.aborted) setDetail({ kind: "entity", value }) })
      .catch(() => { if (!controller.signal.aborted) setDetailError(true) })
      .finally(() => finishDetailRequest(controller))
  }, [beginDetailRequest, finishDetailRequest])

  const openRelationship = useCallback((id: string) => {
    const controller = beginDetailRequest()
    void getRelationship(id, controller.signal)
      .then((value) => { if (!controller.signal.aborted) setDetail({ kind: "relationship", value }) })
      .catch(() => { if (!controller.signal.aborted) setDetailError(true) })
      .finally(() => finishDetailRequest(controller))
  }, [beginDetailRequest, finishDetailRequest])

  const openCommunity = useCallback((id: string) => {
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
  }, [beginDetailRequest, finishDetailRequest])

  const closeDetail = (open: boolean): void => {
    if (open) return
    detailRequest.current?.abort()
    detailRequest.current = null
    setDetail(null)
    setDetailLoading(false)
    setDetailError(false)
  }

  return (
    <section className="flex size-full min-h-0 flex-col" aria-label="Graph Explorer">
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-4">
        <div className="flex items-center gap-2"><Database className="size-4 text-primary" /><h2 className="text-sm font-semibold">Graph Explorer</h2></div>
        {projection !== null ? <Badge variant="success">ready</Badge> : <Badge variant={unavailable ? "destructive" : "outline"}>{unavailable ? "unavailable" : "loading"}</Badge>}
      </header>
      {projection === null && loading ? <div className="space-y-3 p-4"><Skeleton className="h-16" /><Skeleton className="h-80" /></div> : null}
      {projection === null && !loading && unavailable ? <div className="flex flex-1 flex-col items-center justify-center p-6 text-center"><TriangleAlert className="mb-3 size-8 text-warning" /><p className="text-sm font-medium">Graph data unavailable</p><p className="mt-1 text-xs text-muted-foreground">Run GraphLoom index first.</p></div> : null}
      {projection === null && !loading && !unavailable && error !== null ? <div className="flex flex-1 flex-col items-center justify-center p-6 text-center"><TriangleAlert className="mb-3 size-8 text-warning" /><p className="text-sm font-medium">{error}</p><p className="mt-1 text-xs text-muted-foreground">The focused records could not be loaded.</p><Button className="mt-4" size="sm" variant="outline" onClick={loadOverview}>Load overview</Button></div> : null}
      {projection !== null ? (
        <div className="flex min-h-0 flex-1 flex-col p-3">
          {summary !== null ? <><div className="mb-3 grid grid-cols-4 gap-1.5">{[["Entities", summary.entity_count], ["Edges", summary.relationship_count], ["Communities", summary.community_count], ["Reports", summary.community_report_count]].map(([label, count]) => <div key={String(label)} className="rounded-md border bg-card p-2 text-center"><div className="text-base font-semibold">{count}</div><div className="text-[10px] text-muted-foreground">{label}</div></div>)}</div><div className="mb-3 space-y-1.5 text-[10px] text-muted-foreground"><p>Community levels: {summary.community_levels.length > 0 ? summary.community_levels.join(", ") : "none"} · Untyped entities: {summary.untyped_entity_count}</p><div className="flex flex-wrap gap-1">{Object.entries(summary.entity_types).map(([name, count]) => <Badge key={name} variant="outline">{name}: {count}</Badge>)}</div></div></> : null}
          {summary === null && summaryError ? <p className="mb-3 rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-xs text-warning">Graph summary is unavailable. The bounded visualization is still available.</p> : null}
          {summary === null && !summaryError ? <Skeleton className="mb-3 h-16" /> : null}
          <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col">
            <TabsList className="grid w-full grid-cols-4"><TabsTrigger value="graph">Graph</TabsTrigger><TabsTrigger value="entities">Entities</TabsTrigger><TabsTrigger value="relationships">Relations</TabsTrigger><TabsTrigger value="communities">Communities</TabsTrigger></TabsList>
            <TabsContent value="graph" className="flex min-h-0 flex-1"><NetworkPreview projection={projection} mode={mode} loading={loading} error={error} onEntity={openEntity} onRelationship={openRelationship} onBackOverview={loadOverview} onReload={loadOverview} /></TabsContent>
            <TabsContent value="entities" className="flex min-h-0 flex-1"><EntityList onSelect={openEntity} /></TabsContent>
            <TabsContent value="relationships" className="flex min-h-0 flex-1"><RelationshipList onSelect={openRelationship} /></TabsContent>
            <TabsContent value="communities" className="flex min-h-0 flex-1"><CommunityList onSelect={openCommunity} /></TabsContent>
          </Tabs>
        </div>
      ) : null}
      <GraphDetailSheet detail={detail} loading={detailLoading} error={detailError} onOpenChange={closeDetail} onFocusEntity={(id) => loadFocus({ entity_ids: [id], relationship_ids: [], depth: 1, max_entities: 80, max_relationships: 160 })} onFocusRelationship={(id) => loadFocus({ entity_ids: [], relationship_ids: [id], depth: 1, max_entities: 80, max_relationships: 160 })} />
    </section>
  )
}
