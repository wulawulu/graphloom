import { useEffect, useRef, useState } from "react"
import { ChevronRight, Filter } from "lucide-react"
import { useTranslation } from "react-i18next"

import { listCommunities, listEntities, listRelationships } from "@/api/client"
import type { EntityListParameters, GraphCommunity, GraphEntity, GraphRelationship, RelationshipListParameters } from "@/api/types"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { communityDisplayTitle } from "@/lib/graph"

interface EntityListProps { onSelect: (id: string) => void }

type EntitySortValue = "degree-desc" | "degree-asc" | "rank-desc" | "rank-asc" | "title-asc" | "title-desc"
type RelationshipSortValue = "weight-desc" | "weight-asc" | "rank-desc" | "rank-asc" | "source-asc" | "source-desc" | "target-asc" | "target-desc"

const ENTITY_SORT_PARAMETERS = {
  "degree-desc": { sort: "degree", order: "desc" },
  "degree-asc": { sort: "degree", order: "asc" },
  "rank-desc": { sort: "rank", order: "desc" },
  "rank-asc": { sort: "rank", order: "asc" },
  "title-asc": { sort: "title", order: "asc" },
  "title-desc": { sort: "title", order: "desc" },
} as const satisfies Record<EntitySortValue, Pick<EntityListParameters, "sort" | "order">>

const RELATIONSHIP_SORT_PARAMETERS = {
  "weight-desc": { sort: "weight", order: "desc" },
  "weight-asc": { sort: "weight", order: "asc" },
  "rank-desc": { sort: "rank", order: "desc" },
  "rank-asc": { sort: "rank", order: "asc" },
  "source-asc": { sort: "source", order: "asc" },
  "source-desc": { sort: "source", order: "desc" },
  "target-asc": { sort: "target", order: "asc" },
  "target-desc": { sort: "target", order: "desc" },
} as const satisfies Record<RelationshipSortValue, Pick<RelationshipListParameters, "sort" | "order">>

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}

export function EntityList({ onSelect }: EntityListProps): React.ReactElement {
  const { t } = useTranslation()
  const [items, setItems] = useState<GraphEntity[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [draftEntityType, setDraftEntityType] = useState("")
  const [draftCommunity, setDraftCommunity] = useState("")
  const [appliedFilters, setAppliedFilters] = useState({ entityType: "", community: "" })
  const [sort, setSort] = useState<EntitySortValue>("degree-desc")
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)
  const request = useRef<AbortController | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    request.current?.abort(); request.current = controller
    setLoading(true); setError(false)
    void listEntities({ type: appliedFilters.entityType || undefined, community: appliedFilters.community || undefined, limit: 50, ...ENTITY_SORT_PARAMETERS[sort] }, controller.signal)
      .then((response) => { if (request.current === controller && !controller.signal.aborted) { setItems(response.items); setCursor(response.next_cursor) } })
      .catch((reason: unknown) => { if (request.current === controller && !isAbort(reason)) setError(true) })
      .finally(() => { if (request.current === controller) { request.current = null; setLoading(false) } })
    return () => { request.current?.abort(); if (request.current === controller) request.current = null }
  }, [appliedFilters, sort])

  const applyFilters = (): void => {
    request.current?.abort()
    request.current = null
    setItems([])
    setCursor(null)
    setLoading(true)
    setAppliedFilters({ entityType: draftEntityType, community: draftCommunity })
  }

  const loadMore = (): void => {
    if (cursor === null) return
    request.current?.abort()
    const controller = new AbortController(); request.current = controller
    setLoading(true); setError(false)
    void listEntities({ type: appliedFilters.entityType || undefined, community: appliedFilters.community || undefined, limit: 50, after: cursor, ...ENTITY_SORT_PARAMETERS[sort] }, controller.signal)
      .then((response) => { if (request.current === controller && !controller.signal.aborted) { setItems((current) => [...current, ...response.items]); setCursor(response.next_cursor) } })
      .catch((reason: unknown) => { if (request.current === controller && !isAbort(reason)) setError(true) })
      .finally(() => { if (request.current === controller) { request.current = null; setLoading(false) } })
  }

  const changeSort = (value: EntitySortValue): void => {
    request.current?.abort(); request.current = null; setItems([]); setCursor(null); setLoading(true); setSort(value)
  }

  return <GraphListFrame sortControl={<SortControl label={t("graph.labels.entitySort")} value={sort} onChange={(value) => changeSort(value as EntitySortValue)} options={[["degree-desc", t("graph.sort.degreeDescending")], ["degree-asc", t("graph.sort.degreeAscending")], ["rank-desc", t("graph.sort.rankDescending")], ["rank-asc", t("graph.sort.rankAscending")], ["title-asc", t("graph.labels.titleAZ")], ["title-desc", t("graph.labels.titleZA")]]} />} filters={<><Input aria-label={t("graph.labels.entityTypeFilter")} placeholder={t("graph.labels.exactEntityType")} value={draftEntityType} onChange={(event) => setDraftEntityType(event.target.value)} /><Input aria-label={t("graph.labels.entityCommunityFilter")} placeholder={t("graph.labels.exactCommunityId")} value={draftCommunity} onChange={(event) => setDraftCommunity(event.target.value)} /><Button type="submit" variant="outline" size="icon" aria-label={t("graph.actions.applyEntityFilters")}><Filter /></Button></>} onApplyFilters={applyFilters} items={items} cursor={cursor} loading={loading} error={error} onLoadMore={loadMore} render={(item) => <button key={item.id} type="button" className="flex w-full items-center justify-between rounded-md border p-2.5 text-left hover:bg-accent" onClick={() => onSelect(item.id)}><span className="min-w-0"><span className="block truncate text-sm font-medium">{item.title}</span><span className="text-[11px] text-muted-foreground">{item.entity_type ?? t("graph.labels.untyped")} · {t("graph.status.degreeValue", { value: item.degree ?? "—" })} · {t("graph.status.rankValue", { value: item.rank ?? "—" })}</span></span><ChevronRight className="size-4 shrink-0" /></button>} />
}

export function RelationshipList({ onSelect }: EntityListProps): React.ReactElement {
  const { t } = useTranslation()
  const [items, setItems] = useState<GraphRelationship[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [draftSource, setDraftSource] = useState("")
  const [draftTarget, setDraftTarget] = useState("")
  const [appliedFilters, setAppliedFilters] = useState({ source: "", target: "" })
  const [sort, setSort] = useState<RelationshipSortValue>("weight-desc")
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)
  const request = useRef<AbortController | null>(null)
  useEffect(() => {
    const controller = new AbortController(); request.current?.abort(); request.current = controller; setLoading(true); setError(false)
    void listRelationships({ source: appliedFilters.source || undefined, target: appliedFilters.target || undefined, limit: 50, ...RELATIONSHIP_SORT_PARAMETERS[sort] }, controller.signal).then((response) => { if (request.current === controller && !controller.signal.aborted) { setItems(response.items); setCursor(response.next_cursor) } }).catch((reason: unknown) => { if (request.current === controller && !isAbort(reason)) setError(true) }).finally(() => { if (request.current === controller) { request.current = null; setLoading(false) } })
    return () => { request.current?.abort(); if (request.current === controller) request.current = null }
  }, [appliedFilters, sort])
  const applyFilters = (): void => { request.current?.abort(); request.current = null; setItems([]); setCursor(null); setLoading(true); setAppliedFilters({ source: draftSource, target: draftTarget }) }
  const loadMore = (): void => { if (cursor === null) return; request.current?.abort(); const controller = new AbortController(); request.current = controller; setLoading(true); setError(false); void listRelationships({ source: appliedFilters.source || undefined, target: appliedFilters.target || undefined, limit: 50, after: cursor, ...RELATIONSHIP_SORT_PARAMETERS[sort] }, controller.signal).then((response) => { if (request.current === controller && !controller.signal.aborted) { setItems((current) => [...current, ...response.items]); setCursor(response.next_cursor) } }).catch((reason: unknown) => { if (request.current === controller && !isAbort(reason)) setError(true) }).finally(() => { if (request.current === controller) { request.current = null; setLoading(false) } }) }
  const changeSort = (value: RelationshipSortValue): void => { request.current?.abort(); request.current = null; setItems([]); setCursor(null); setLoading(true); setSort(value) }
  return <GraphListFrame sortControl={<SortControl label={t("graph.labels.relationshipSort")} value={sort} onChange={(value) => changeSort(value as RelationshipSortValue)} options={[["weight-desc", t("graph.sort.weightDescending")], ["weight-asc", t("graph.sort.weightAscending")], ["rank-desc", t("graph.sort.rankDescending")], ["rank-asc", t("graph.sort.rankAscending")], ["source-asc", t("graph.labels.sourceAZ")], ["source-desc", t("graph.labels.sourceZA")], ["target-asc", t("graph.labels.targetAZ")], ["target-desc", t("graph.labels.targetZA")]]} />} filters={<><Input aria-label={t("graph.labels.relationshipSourceFilter")} placeholder={t("graph.labels.exactSourceTitle")} value={draftSource} onChange={(event) => setDraftSource(event.target.value)} /><Input aria-label={t("graph.labels.relationshipTargetFilter")} placeholder={t("graph.labels.exactTargetTitle")} value={draftTarget} onChange={(event) => setDraftTarget(event.target.value)} /><Button type="submit" variant="outline" size="icon" aria-label={t("graph.actions.applyRelationshipFilters")}><Filter /></Button></>} onApplyFilters={applyFilters} items={items} cursor={cursor} loading={loading} error={error} onLoadMore={loadMore} render={(item) => <button key={item.id} type="button" className="flex w-full items-center justify-between rounded-md border p-2.5 text-left hover:bg-accent" onClick={() => onSelect(item.id)}><span className="min-w-0"><span className="block truncate text-sm font-medium">{item.source} → {item.target}</span><span className="text-[11px] text-muted-foreground">{t("graph.status.weightValue", { value: item.weight ?? "—" })} · {t("graph.status.rankValue", { value: item.rank ?? "—" })}</span></span><ChevronRight className="size-4 shrink-0" /></button>} />
}

export function CommunityList({ onSelect }: EntityListProps): React.ReactElement {
  const { t } = useTranslation()
  const [items, setItems] = useState<GraphCommunity[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [draftLevel, setDraftLevel] = useState("")
  const [draftParent, setDraftParent] = useState("")
  const [appliedFilters, setAppliedFilters] = useState<{ level: number | undefined; parent: number | undefined }>({ level: undefined, parent: undefined })
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)
  const request = useRef<AbortController | null>(null)
  const parsed = (value: string): number | undefined => value === "" ? undefined : Number(value)
  useEffect(() => {
    const controller = new AbortController(); request.current?.abort(); request.current = controller; setLoading(true); setError(false)
    void listCommunities({ level: appliedFilters.level, parent: appliedFilters.parent, limit: 50 }, controller.signal).then((response) => { if (request.current === controller && !controller.signal.aborted) { setItems(response.items); setCursor(response.next_cursor) } }).catch((reason: unknown) => { if (request.current === controller && !isAbort(reason)) setError(true) }).finally(() => { if (request.current === controller) { request.current = null; setLoading(false) } })
    return () => { request.current?.abort(); if (request.current === controller) request.current = null }
  }, [appliedFilters])
  const applyFilters = (): void => { request.current?.abort(); request.current = null; setItems([]); setCursor(null); setLoading(true); setAppliedFilters({ level: parsed(draftLevel), parent: parsed(draftParent) }) }
  const loadMore = (): void => { if (cursor === null) return; request.current?.abort(); const controller = new AbortController(); request.current = controller; setLoading(true); setError(false); void listCommunities({ level: appliedFilters.level, parent: appliedFilters.parent, limit: 50, after: cursor }, controller.signal).then((response) => { if (request.current === controller && !controller.signal.aborted) { setItems((current) => [...current, ...response.items]); setCursor(response.next_cursor) } }).catch((reason: unknown) => { if (request.current === controller && !isAbort(reason)) setError(true) }).finally(() => { if (request.current === controller) { request.current = null; setLoading(false) } }) }
  return <GraphListFrame filters={<><Input type="number" aria-label={t("graph.labels.communityLevelFilter")} placeholder={t("explainability.labels.level")} value={draftLevel} onChange={(event) => setDraftLevel(event.target.value)} /><Input type="number" aria-label={t("graph.labels.communityParentFilter")} placeholder={t("graph.labels.parent")} value={draftParent} onChange={(event) => setDraftParent(event.target.value)} /><Button type="submit" variant="outline" size="icon" aria-label={t("graph.actions.applyCommunityFilters")}><Filter /></Button></>} onApplyFilters={applyFilters} items={items} cursor={cursor} loading={loading} error={error} onLoadMore={loadMore} render={(item) => { const title = communityDisplayTitle(item, t("graph.community.communityNumber", { shortId: item.short_id })); return <button key={item.id} type="button" className="flex w-full items-center justify-between rounded-md border p-2.5 text-left hover:bg-accent" aria-label={t("graph.navigation.openCommunity", { title })} onClick={() => onSelect(item.id)}><span className="min-w-0"><span className="line-clamp-2 block break-words text-sm font-medium">{title}</span><span className="block text-[10px] text-muted-foreground">{t("graph.community.referenceMetadata", { shortId: item.short_id, level: item.level })}</span><span className="line-clamp-2 text-[11px] text-muted-foreground">{item.report?.summary ?? t("graph.labels.noReportSummary")}</span></span><ChevronRight className="size-4 shrink-0" /></button> }} />
}

function SortControl({ label, value, onChange, options }: { label: string; value: string; onChange: (value: string) => void; options: Array<[string, string]> }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="flex items-center gap-2"><span className="text-xs text-muted-foreground">{t("graph.labels.sort")}</span><Select value={value} onValueChange={onChange}><SelectTrigger className="h-8 min-w-32 flex-1" aria-label={label}><SelectValue /></SelectTrigger><SelectContent>{options.map(([option, title]) => <SelectItem key={option} value={option}>{title}</SelectItem>)}</SelectContent></Select></div>
}

interface GraphListFrameProps<T> { sortControl?: React.ReactNode; filters: React.ReactNode; onApplyFilters: () => void; items: T[]; cursor: string | null; loading: boolean; error: boolean; onLoadMore: () => void; render: (item: T) => React.ReactNode }
function GraphListFrame<T>({ sortControl, filters, onApplyFilters, items, cursor, loading, error, onLoadMore, render }: GraphListFrameProps<T>): React.ReactElement {
  const { t } = useTranslation()
  const submit = (event: React.FormEvent<HTMLFormElement>): void => { event.preventDefault(); onApplyFilters() }
  return <div className="flex min-h-0 flex-1 flex-col gap-3">{sortControl}<form className="grid grid-cols-[1fr_1fr_auto] gap-2" onSubmit={submit}>{filters}</form>{error ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-xs text-red-300">{t("graph.messages.graphListIsUnavailable")}</p> : null}<ScrollArea className="min-h-0 flex-1"><div className="space-y-1.5 pr-2">{items.map(render)}{!loading && !error && items.length === 0 ? <p className="py-12 text-center text-xs text-muted-foreground">{t("graph.messages.noGraphItemsMatchThisFilter")}</p> : null}{cursor !== null ? <Button variant="outline" size="sm" className="w-full" disabled={loading} onClick={onLoadMore}>{t("runs.actions.loadMore")}</Button> : null}</div></ScrollArea></div>
}
