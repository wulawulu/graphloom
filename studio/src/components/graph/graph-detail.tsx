import { useState } from "react"
import { ArrowDown, Building2, Check, Copy, GitBranch, Network, UsersRound, X } from "lucide-react"

import type { GraphCommunity, GraphCommunityReportDetail, GraphEntityDetail, GraphRelationshipDetail } from "@/api/types"
import { SafeMarkdown } from "@/components/content/safe-markdown"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"

export type GraphDetail =
  | { kind: "entity"; value: GraphEntityDetail }
  | { kind: "relationship"; value: GraphRelationshipDetail }
  | { kind: "community"; value: GraphCommunity; report: GraphCommunityReportDetail | null }

interface GraphInspectorProps {
  detail: GraphDetail | null
  loading: boolean
  error: boolean
  onClear: () => void
  onFocusEntity: (id: string) => void
  onFocusRelationship: (id: string) => void
}

export function GraphInspector(props: GraphInspectorProps): React.ReactElement {
  const { detail } = props
  const title = detail === null
    ? "Graph item"
    : detail.kind === "relationship"
      ? "Relationship"
      : detail.value.title
  return (
    <section className="flex size-full min-h-0 flex-col" aria-label="Graph Inspector" tabIndex={-1} onKeyDown={(event) => { if (event.key === "Escape") props.onClear() }}>
      <header className="flex h-11 shrink-0 items-center justify-between border-b px-3">
        <div className="min-w-0"><h2 className="truncate text-sm font-semibold">{props.loading ? "Loading graph detail" : title}</h2><p className="text-[10px] text-muted-foreground">{detail?.kind ?? "Inspector"}</p></div>
        {detail !== null || props.loading || props.error ? <Button variant="ghost" size="icon" className="size-8" aria-label="Clear graph selection" onClick={props.onClear}><X /></Button> : null}
      </header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="p-3">
          {detail === null && !props.loading && !props.error ? <div className="flex min-h-64 flex-col items-center justify-center px-5 text-center"><Network className="mb-3 size-8 text-muted-foreground/40" /><p className="text-sm font-medium">Select a graph object</p><p className="mt-1 text-xs leading-5 text-muted-foreground">Select a node, relationship, or community to inspect it.</p></div> : null}
          {props.loading ? <p className="py-10 text-center text-sm text-muted-foreground">Loading structured detail…</p> : null}
          {props.error ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-red-300">Graph detail is unavailable.</p> : null}
          {detail?.kind === "entity" ? <EntityDetail value={detail.value} onFocus={props.onFocusEntity} /> : null}
          {detail?.kind === "relationship" ? <RelationshipDetail value={detail.value} onFocus={props.onFocusRelationship} /> : null}
          {detail?.kind === "community" ? <CommunityDetail value={detail.value} report={detail.report} /> : null}
          {detail !== null ? <RawData value={detail} /> : null}
        </div>
      </ScrollArea>
    </section>
  )
}

function RawData({ value }: { value: GraphDetail }): React.ReactElement {
  return <details className="rounded-md border bg-muted/20 p-3"><summary className="cursor-pointer text-xs font-semibold text-muted-foreground">Developer · Raw JSON</summary><pre className="mt-3 overflow-x-auto whitespace-pre-wrap break-all text-[10px] leading-4">{JSON.stringify(value, null, 2)}</pre></details>
}

function Section({ title, children }: { title: string; children: React.ReactNode }): React.ReactElement {
  return <section className="space-y-2"><h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h3>{children}</section>
}

function IdBadges({ values }: { values: string[] }): React.ReactElement {
  return <div className="flex flex-wrap gap-1">{values.length === 0 ? <span className="text-sm text-muted-foreground">None</span> : values.map((value) => <Badge key={value} variant="outline">{value}</Badge>)}</div>
}

function Metadata({ id, shortId }: { id: string; shortId?: string | null }): React.ReactElement {
  const [copied, setCopied] = useState(false)
  const copy = (): void => {
    void navigator.clipboard.writeText(id).then(() => setCopied(true)).catch(() => setCopied(false))
  }
  return (
    <details className="rounded-md border bg-muted/30 p-3 text-xs">
      <summary className="cursor-pointer font-medium text-muted-foreground">Metadata</summary>
      <div className="mt-3 space-y-2">
        {shortId !== undefined && shortId !== null ? <p><span className="text-muted-foreground">Short ID:</span> {shortId}</p> : null}
        <div className="flex items-center gap-2"><code className="min-w-0 flex-1 truncate">{id}</code><Button variant="ghost" size="icon" aria-label="Copy ID" onClick={copy}>{copied ? <Check /> : <Copy />}</Button></div>
      </div>
    </details>
  )
}

function SourceIds({ values }: { values: string[] }): React.ReactElement {
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? values : values.slice(0, 20)
  return (
    <details className="rounded-md border p-3">
      <summary className="cursor-pointer text-sm font-medium">{values.length} source text units</summary>
      <div className="mt-3 space-y-2">
        <IdBadges values={visible} />
        {!showAll && values.length > visible.length ? <Button variant="ghost" size="sm" onClick={() => setShowAll(true)}>Show {values.length - visible.length} more</Button> : null}
      </div>
    </details>
  )
}

function EntityDetail({ value, onFocus }: { value: GraphEntityDetail; onFocus: (id: string) => void }): React.ReactElement {
  return (
    <div className="space-y-5 pb-5">
      <div className="flex items-start justify-between gap-3 rounded-lg border bg-card p-4">
        <div className="flex min-w-0 gap-3"><div className="rounded-full bg-primary/10 p-2 text-primary"><Building2 className="size-5" /></div><div><h3 className="truncate font-semibold">{value.title}</h3><div className="mt-1 flex gap-1"><Badge variant="outline">{value.entity_type ?? "Untyped"}</Badge><Badge variant="outline">Degree {value.degree ?? "—"}</Badge><Badge variant="outline">Rank {value.rank ?? "—"}</Badge></div></div></div>
        <Button size="sm" onClick={() => onFocus(value.id)}><Network /> Focus neighborhood</Button>
      </div>
      <Section title="Description"><p className="whitespace-pre-wrap text-sm leading-6">{value.description ?? "No description"}</p></Section>
      <Separator />
      <Section title="Communities"><IdBadges values={value.community_ids} /></Section>
      <Section title="Sources"><SourceIds values={value.text_unit_ids} /></Section>
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}

function RelationshipDetail({ value, onFocus }: { value: GraphRelationshipDetail; onFocus: (id: string) => void }): React.ReactElement {
  return (
    <div className="space-y-5 pb-5">
      <div className="rounded-lg border bg-card p-4 text-center">
        <div className="font-semibold">{value.source}</div><ArrowDown className="mx-auto my-2 size-5 text-primary" /><div className="font-semibold">{value.target}</div>
        <div className="mt-3 flex justify-center gap-1"><Badge variant="outline">Weight {value.weight ?? "—"}</Badge><Badge variant="outline">Rank {value.rank ?? "—"}</Badge></div>
      </div>
      <Button className="w-full" onClick={() => onFocus(value.id)}><GitBranch /> Focus relationship</Button>
      <Section title="Description"><p className="whitespace-pre-wrap text-sm leading-6">{value.description ?? "No description"}</p></Section>
      <Section title="Sources"><SourceIds values={value.text_unit_ids} /></Section>
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}

function CommunityDetail({ value, report }: { value: GraphCommunity; report: GraphCommunityReportDetail | null }): React.ReactElement {
  return (
    <div className="space-y-5 pb-5">
      <div className="flex gap-3 rounded-lg border bg-card p-4"><div className="rounded-full bg-primary/10 p-2 text-primary"><UsersRound className="size-5" /></div><div><h3 className="font-semibold">{value.title}</h3><div className="mt-1 flex gap-1"><Badge variant="outline">Level {value.level}</Badge><Badge variant="outline">Short ID {value.short_id}</Badge></div></div></div>
      <Section title="Summary"><p className="whitespace-pre-wrap text-sm leading-6">{value.report?.summary ?? "No report summary"}</p></Section>
      <Section title="Hierarchy"><p className="text-sm"><span className="text-muted-foreground">Parent:</span> {value.parent}</p><p className="text-sm"><span className="text-muted-foreground">Children:</span> {value.children.join(", ") || "None"}</p></Section>
      {report !== null ? <Section title="Report"><div className="mb-2 flex items-center gap-2"><GitBranch className="size-4 text-primary" /><span className="font-medium">{report.title}</span>{report.rank !== null ? <Badge variant="outline">Rank {report.rank}</Badge> : null}</div><SafeMarkdown>{report.full_content}</SafeMarkdown></Section> : null}
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}
