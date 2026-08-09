import type { GraphCommunity, GraphCommunityReportDetail, GraphEntityDetail, GraphRelationshipDetail } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"

export type GraphDetail =
  | { kind: "entity"; value: GraphEntityDetail }
  | { kind: "relationship"; value: GraphRelationshipDetail }
  | { kind: "community"; value: GraphCommunity; report: GraphCommunityReportDetail | null }

interface GraphDetailSheetProps { detail: GraphDetail | null; loading: boolean; error: boolean; onOpenChange: (open: boolean) => void }

export function GraphDetailSheet({ detail, loading, error, onOpenChange }: GraphDetailSheetProps): React.ReactElement {
  const title = detail === null
    ? "Graph item"
    : detail.kind === "relationship"
      ? `${detail.value.source} → ${detail.value.target}`
      : detail.value.title
  return (
    <Sheet open={detail !== null || loading || error} onOpenChange={onOpenChange}>
      <SheetContent>
        <SheetHeader><SheetTitle>{loading ? "Loading graph detail" : title}</SheetTitle><SheetDescription>{detail === null ? "Fetching the selected graph record." : detail.kind}</SheetDescription></SheetHeader>
        <ScrollArea className="min-h-0 flex-1 pr-3">
          {error ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-red-300">Graph detail is unavailable.</p> : null}
          {detail?.kind === "entity" ? <EntityDetail value={detail.value} /> : null}
          {detail?.kind === "relationship" ? <RelationshipDetail value={detail.value} /> : null}
          {detail?.kind === "community" ? <CommunityDetail value={detail.value} report={detail.report} /> : null}
        </ScrollArea>
      </SheetContent>
    </Sheet>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }): React.ReactElement {
  return <div className="space-y-1"><dt className="font-mono text-[10px] tracking-wide text-muted-foreground uppercase">{label}</dt><dd className="whitespace-pre-wrap text-sm leading-6">{children}</dd></div>
}

function IdBadges({ values }: { values: string[] }): React.ReactElement {
  return <div className="flex flex-wrap gap-1">{values.length === 0 ? <span className="text-muted-foreground">None</span> : values.map((value) => <Badge key={value} variant="outline">{value}</Badge>)}</div>
}

function EntityDetail({ value }: { value: GraphEntityDetail }): React.ReactElement {
  return <dl className="space-y-4"><Field label="ID">{value.id}</Field><Field label="Type">{value.entity_type ?? "Untyped"}</Field><Field label="Rank">{value.rank ?? "—"}</Field><Field label="Description">{value.description ?? "No description"}</Field><Field label="Communities"><IdBadges values={value.community_ids} /></Field><Field label="Text units"><IdBadges values={value.text_unit_ids} /></Field></dl>
}

function RelationshipDetail({ value }: { value: GraphRelationshipDetail }): React.ReactElement {
  return <dl className="space-y-4"><Field label="ID">{value.id}</Field><Field label="Source title">{value.source}</Field><Field label="Target title">{value.target}</Field><Field label="Weight">{value.weight ?? "—"}</Field><Field label="Rank">{value.rank ?? "—"}</Field><Field label="Description">{value.description ?? "No description"}</Field><Field label="Text units"><IdBadges values={value.text_unit_ids} /></Field></dl>
}

function CommunityDetail({ value, report }: { value: GraphCommunity; report: GraphCommunityReportDetail | null }): React.ReactElement {
  return <dl className="space-y-4"><Field label="ID">{value.id}</Field><Field label="Short ID">{value.short_id}</Field><Field label="Level / Parent">{value.level} / {value.parent}</Field><Field label="Children">{value.children.join(", ") || "None"}</Field><Field label="Report summary">{value.report?.summary ?? "No report"}</Field>{report !== null ? <><Field label="Report title">{report.title}</Field><Field label="Rank">{report.rank ?? "—"}</Field><Field label="Full report">{report.full_content}</Field></> : null}</dl>
}
