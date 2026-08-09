import { useEffect, useRef, useState } from "react"
import { ChevronRight, Filter } from "lucide-react"

import { listCommunities, listEntities, listRelationships } from "@/api/client"
import type { GraphCommunity, GraphEntity, GraphRelationship } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"

interface EntityListProps { onSelect: (id: string) => void }

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}

export function EntityList({ onSelect }: EntityListProps): React.ReactElement {
  const [items, setItems] = useState<GraphEntity[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [entityType, setEntityType] = useState("")
  const [community, setCommunity] = useState("")
  const [revision, setRevision] = useState(0)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)
  const request = useRef<AbortController | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    request.current?.abort(); request.current = controller
    setLoading(true); setError(false)
    void listEntities({ type: entityType || undefined, community: community || undefined, limit: 50 }, controller.signal)
      .then((response) => { setItems(response.items); setCursor(response.next_cursor) })
      .catch((reason: unknown) => { if (!isAbort(reason)) setError(true) })
      .finally(() => { if (!controller.signal.aborted) setLoading(false) })
    return () => { request.current?.abort(); if (request.current === controller) request.current = null }
  }, [community, entityType, revision])

  const loadMore = (): void => {
    if (cursor === null) return
    request.current?.abort()
    const controller = new AbortController(); request.current = controller
    setLoading(true); setError(false)
    void listEntities({ type: entityType || undefined, community: community || undefined, limit: 50, after: cursor }, controller.signal)
      .then((response) => { setItems((current) => [...current, ...response.items]); setCursor(response.next_cursor) })
      .catch((reason: unknown) => { if (!isAbort(reason)) setError(true) })
      .finally(() => { if (!controller.signal.aborted) { request.current = null; setLoading(false) } })
  }

  return <GraphListFrame filters={<><Input aria-label="Entity type filter" placeholder="Exact entity type" value={entityType} onChange={(event) => setEntityType(event.target.value)} /><Input aria-label="Entity community filter" placeholder="Exact community ID" value={community} onChange={(event) => setCommunity(event.target.value)} /><Button variant="outline" size="icon" aria-label="Apply entity filters" onClick={() => setRevision((value) => value + 1)}><Filter /></Button></>} items={items} cursor={cursor} loading={loading} error={error} onLoadMore={loadMore} render={(item) => <button key={item.id} type="button" className="flex w-full items-center justify-between rounded-md border p-2.5 text-left hover:bg-accent" onClick={() => onSelect(item.id)}><span className="min-w-0"><span className="block truncate text-sm font-medium">{item.title}</span><span className="text-[11px] text-muted-foreground">{item.entity_type ?? "Untyped"} · rank {item.rank ?? "—"}</span></span><ChevronRight className="size-4 shrink-0" /></button>} />
}

export function RelationshipList({ onSelect }: EntityListProps): React.ReactElement {
  const [items, setItems] = useState<GraphRelationship[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [source, setSource] = useState("")
  const [target, setTarget] = useState("")
  const [revision, setRevision] = useState(0)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)
  const request = useRef<AbortController | null>(null)
  useEffect(() => {
    const controller = new AbortController(); request.current?.abort(); request.current = controller; setLoading(true); setError(false)
    void listRelationships({ source: source || undefined, target: target || undefined, limit: 50 }, controller.signal).then((response) => { setItems(response.items); setCursor(response.next_cursor) }).catch((reason: unknown) => { if (!isAbort(reason)) setError(true) }).finally(() => { if (!controller.signal.aborted) setLoading(false) })
    return () => { request.current?.abort(); if (request.current === controller) request.current = null }
  }, [revision, source, target])
  const loadMore = (): void => { if (cursor === null) return; request.current?.abort(); const controller = new AbortController(); request.current = controller; setLoading(true); setError(false); void listRelationships({ source: source || undefined, target: target || undefined, limit: 50, after: cursor }, controller.signal).then((response) => { setItems((current) => [...current, ...response.items]); setCursor(response.next_cursor) }).catch((reason: unknown) => { if (!isAbort(reason)) setError(true) }).finally(() => { if (!controller.signal.aborted) { request.current = null; setLoading(false) } }) }
  return <GraphListFrame filters={<><Input aria-label="Relationship source filter" placeholder="Exact source title" value={source} onChange={(event) => setSource(event.target.value)} /><Input aria-label="Relationship target filter" placeholder="Exact target title" value={target} onChange={(event) => setTarget(event.target.value)} /><Button variant="outline" size="icon" aria-label="Apply relationship filters" onClick={() => setRevision((value) => value + 1)}><Filter /></Button></>} items={items} cursor={cursor} loading={loading} error={error} onLoadMore={loadMore} render={(item) => <button key={item.id} type="button" className="flex w-full items-center justify-between rounded-md border p-2.5 text-left hover:bg-accent" onClick={() => onSelect(item.id)}><span className="min-w-0"><span className="block truncate text-sm font-medium">{item.source} → {item.target}</span><span className="text-[11px] text-muted-foreground">weight {item.weight ?? "—"} · rank {item.rank ?? "—"}</span></span><ChevronRight className="size-4 shrink-0" /></button>} />
}

export function CommunityList({ onSelect }: EntityListProps): React.ReactElement {
  const [items, setItems] = useState<GraphCommunity[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [level, setLevel] = useState("")
  const [parent, setParent] = useState("")
  const [revision, setRevision] = useState(0)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)
  const request = useRef<AbortController | null>(null)
  const parsed = (value: string): number | undefined => value === "" ? undefined : Number(value)
  useEffect(() => {
    const controller = new AbortController(); request.current?.abort(); request.current = controller; setLoading(true); setError(false)
    void listCommunities({ level: parsed(level), parent: parsed(parent), limit: 50 }, controller.signal).then((response) => { setItems(response.items); setCursor(response.next_cursor) }).catch((reason: unknown) => { if (!isAbort(reason)) setError(true) }).finally(() => { if (!controller.signal.aborted) setLoading(false) })
    return () => { request.current?.abort(); if (request.current === controller) request.current = null }
  }, [level, parent, revision])
  const loadMore = (): void => { if (cursor === null) return; request.current?.abort(); const controller = new AbortController(); request.current = controller; setLoading(true); setError(false); void listCommunities({ level: parsed(level), parent: parsed(parent), limit: 50, after: cursor }, controller.signal).then((response) => { setItems((current) => [...current, ...response.items]); setCursor(response.next_cursor) }).catch((reason: unknown) => { if (!isAbort(reason)) setError(true) }).finally(() => { if (!controller.signal.aborted) { request.current = null; setLoading(false) } }) }
  return <GraphListFrame filters={<><Input type="number" aria-label="Community level filter" placeholder="Level" value={level} onChange={(event) => setLevel(event.target.value)} /><Input type="number" aria-label="Community parent filter" placeholder="Parent" value={parent} onChange={(event) => setParent(event.target.value)} /><Button variant="outline" size="icon" aria-label="Apply community filters" onClick={() => setRevision((value) => value + 1)}><Filter /></Button></>} items={items} cursor={cursor} loading={loading} error={error} onLoadMore={loadMore} render={(item) => <button key={item.id} type="button" className="flex w-full items-center justify-between rounded-md border p-2.5 text-left hover:bg-accent" onClick={() => onSelect(item.id)}><span className="min-w-0"><span className="flex items-center gap-2 truncate text-sm font-medium">{item.title}<Badge variant="outline">L{item.level}</Badge></span><span className="block text-[10px] text-muted-foreground">short {item.short_id} · parent {item.parent} · children {item.children.join(", ") || "none"}</span><span className="line-clamp-1 text-[11px] text-muted-foreground">{item.report?.summary ?? "No report summary"}</span></span><ChevronRight className="size-4 shrink-0" /></button>} />
}

interface GraphListFrameProps<T> { filters: React.ReactNode; items: T[]; cursor: string | null; loading: boolean; error: boolean; onLoadMore: () => void; render: (item: T) => React.ReactNode }
function GraphListFrame<T>({ filters, items, cursor, loading, error, onLoadMore, render }: GraphListFrameProps<T>): React.ReactElement {
  return <div className="flex min-h-0 flex-1 flex-col gap-3"><div className="grid grid-cols-[1fr_1fr_auto] gap-2">{filters}</div>{error ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-xs text-red-300">Graph list is unavailable.</p> : null}<ScrollArea className="min-h-0 flex-1"><div className="space-y-1.5 pr-2">{items.map(render)}{!loading && !error && items.length === 0 ? <p className="py-12 text-center text-xs text-muted-foreground">No graph items match this filter.</p> : null}{cursor !== null ? <Button variant="outline" size="sm" className="w-full" disabled={loading} onClick={onLoadMore}>Load more</Button> : null}</div></ScrollArea></div>
}
