import { useCallback, useEffect, useRef, useState } from "react"
import { Database, TriangleAlert } from "lucide-react"

import { ApiError, getCommunity, getCommunityReport, getEntity, getGraphSummary, getRelationship, listEntities, listRelationships } from "@/api/client"
import type { GraphEntity, GraphRelationship, GraphSummary } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

import { GraphDetailSheet, type GraphDetail } from "./graph-detail"
import { CommunityList, EntityList, RelationshipList } from "./graph-lists"
import { NetworkPreview } from "./network-preview"

interface GraphExplorerProps { entityHighlights: ReadonlySet<string>; relationshipHighlights: ReadonlySet<string>; focusRevision: number }

export function GraphExplorer({ entityHighlights, relationshipHighlights, focusRevision }: GraphExplorerProps): React.ReactElement {
  const [summary, setSummary] = useState<GraphSummary | null>(null)
  const [entities, setEntities] = useState<GraphEntity[]>([])
  const [relationships, setRelationships] = useState<GraphRelationship[]>([])
  const [unavailable, setUnavailable] = useState(false)
  const [loading, setLoading] = useState(true)
  const [tab, setTab] = useState("graph")
  const [detail, setDetail] = useState<GraphDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState(false)
  const detailRequest = useRef<AbortController | null>(null)

  useEffect(() => { if (focusRevision > 0) setTab("graph") }, [focusRevision])
  useEffect(() => {
    const controller = new AbortController()
    setLoading(true)
    void Promise.all([
      getGraphSummary(controller.signal),
      listEntities({ limit: 100 }, controller.signal),
      listRelationships({ limit: 200 }, controller.signal),
    ]).then(([nextSummary, entityPage, relationshipPage]) => {
      setSummary(nextSummary); setEntities(entityPage.items); setRelationships(relationshipPage.items); setUnavailable(false)
    }).catch(() => {
      if (!controller.signal.aborted) setUnavailable(true)
    }).finally(() => { if (!controller.signal.aborted) setLoading(false) })
    return () => controller.abort()
  }, [])

  useEffect(() => () => detailRequest.current?.abort(), [])

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
      getCommunityReport(id, controller.signal).catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") throw error
        if (error instanceof ApiError && error.status === 404) return null
        throw error
      }),
    ])
      .then(([value, report]) => { if (!controller.signal.aborted) setDetail({ kind: "community", value, report }) })
      .catch(() => { if (!controller.signal.aborted) setDetailError(true) })
      .finally(() => finishDetailRequest(controller))
  }, [beginDetailRequest, finishDetailRequest])

  return (
    <section className="flex size-full min-h-0 flex-col" aria-label="Graph Explorer">
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-4"><div className="flex items-center gap-2"><Database className="size-4 text-primary" /><h2 className="text-sm font-semibold">Graph Explorer</h2></div>{summary !== null ? <Badge variant="success">ready</Badge> : <Badge variant={unavailable ? "destructive" : "outline"}>{unavailable ? "unavailable" : "loading"}</Badge>}</header>
      {loading ? <div className="space-y-3 p-4"><Skeleton className="h-16" /><Skeleton className="h-80" /></div> : null}
      {!loading && unavailable ? <div className="flex flex-1 flex-col items-center justify-center p-6 text-center"><TriangleAlert className="mb-3 size-8 text-warning" /><p className="text-sm font-medium">Graph data unavailable</p><p className="mt-1 text-xs text-muted-foreground">Run GraphLoom index first.</p></div> : null}
      {!loading && !unavailable && summary !== null ? (
        <div className="flex min-h-0 flex-1 flex-col p-3">
          <div className="mb-3 grid grid-cols-4 gap-1.5">{[["Entities", summary.entity_count], ["Edges", summary.relationship_count], ["Communities", summary.community_count], ["Reports", summary.community_report_count]].map(([label, count]) => <div key={String(label)} className="rounded-md border bg-card p-2 text-center"><div className="text-base font-semibold">{count}</div><div className="text-[10px] text-muted-foreground">{label}</div></div>)}</div>
          <div className="mb-3 space-y-1.5 text-[10px] text-muted-foreground">
            <p>Community levels: {summary.community_levels.length > 0 ? summary.community_levels.join(", ") : "none"} · Untyped entities: {summary.untyped_entity_count}</p>
            <div className="flex flex-wrap gap-1">{Object.entries(summary.entity_types).map(([name, count]) => <Badge key={name} variant="outline">{name}: {count}</Badge>)}</div>
          </div>
          <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col">
            <TabsList className="grid w-full grid-cols-4"><TabsTrigger value="graph">Graph</TabsTrigger><TabsTrigger value="entities">Entities</TabsTrigger><TabsTrigger value="relationships">Relations</TabsTrigger><TabsTrigger value="communities">Communities</TabsTrigger></TabsList>
            <TabsContent value="graph" className="flex min-h-0 flex-1"><NetworkPreview entities={entities} relationships={relationships} totalEntities={summary.entity_count} entityHighlights={entityHighlights} relationshipHighlights={relationshipHighlights} onEntity={openEntity} onRelationship={openRelationship} /></TabsContent>
            <TabsContent value="entities" className="flex min-h-0 flex-1"><EntityList onSelect={openEntity} /></TabsContent>
            <TabsContent value="relationships" className="flex min-h-0 flex-1"><RelationshipList onSelect={openRelationship} /></TabsContent>
            <TabsContent value="communities" className="flex min-h-0 flex-1"><CommunityList onSelect={openCommunity} /></TabsContent>
          </Tabs>
        </div>
      ) : null}
      <GraphDetailSheet detail={detail} loading={detailLoading} error={detailError} onOpenChange={(open) => { if (!open) { detailRequest.current?.abort(); detailRequest.current = null; setDetail(null); setDetailLoading(false); setDetailError(false) } }} />
    </section>
  )
}
