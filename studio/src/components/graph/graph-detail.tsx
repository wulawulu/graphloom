import { useState } from "react"
import { ArrowDown, Building2, Check, Copy, GitBranch, Network, UsersRound, X } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { GraphCommunity, GraphCommunityReportDetail, GraphEntityDetail, GraphRelationshipDetail } from "@/api/types"
import { SafeMarkdown } from "@/components/content/safe-markdown"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import type { ExplainabilityRecordView } from "@/lib/semantic-timeline"

export type GraphDetail =
  | { kind: "entity"; value: GraphEntityDetail }
  | { kind: "relationship"; value: GraphRelationshipDetail }
  | { kind: "community"; value: GraphCommunity; report: GraphCommunityReportDetail | null }

interface GraphInspectorProps {
  detail: GraphDetail | null
  decision?: ExplainabilityRecordView | null
  loading: boolean
  error: boolean
  onClear: () => void
  onFocusEntity: (id: string) => void
  onFocusRelationship: (id: string) => void
}

export function GraphInspector(props: GraphInspectorProps): React.ReactElement {
  const { t } = useTranslation()
  const { detail } = props
  const title = detail === null
    ? t("Graph item")
    : detail.kind === "relationship"
      ? t("Relationship")
      : detail.value.title
  return (
    <section className="flex size-full min-h-0 flex-col" aria-label={t("Graph Inspector")} tabIndex={-1} onKeyDown={(event) => { if (event.key === "Escape") props.onClear() }}>
      <header className="flex h-11 shrink-0 items-center justify-between border-b px-3">
        <div className="min-w-0"><h2 className="truncate text-sm font-semibold">{props.loading ? t("Loading graph detail") : title}</h2><p className="text-[10px] text-muted-foreground">{detail === null ? t("Inspector") : t(detail.kind)}</p></div>
        {detail !== null || props.loading || props.error ? <Button variant="ghost" size="icon" className="size-8" aria-label={t("Clear graph selection")} onClick={props.onClear}><X /></Button> : null}
      </header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="p-3">
          {detail === null && !props.loading && !props.error ? <div className="flex min-h-64 flex-col items-center justify-center px-5 text-center"><Network className="mb-3 size-8 text-muted-foreground/40" /><p className="text-sm font-medium">{t("Select a graph object")}</p><p className="mt-1 text-xs leading-5 text-muted-foreground">{t("Select a node, relationship, or community to inspect it.")}</p></div> : null}
          {props.loading ? <p className="py-10 text-center text-sm text-muted-foreground">{t("Loading structured detail…")}</p> : null}
          {props.error ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-red-300">{t("Graph detail is unavailable.")}</p> : null}
          {detail?.kind === "entity" ? <EntityDetail value={detail.value} onFocus={props.onFocusEntity} /> : null}
          {detail?.kind === "relationship" ? <RelationshipDetail value={detail.value} onFocus={props.onFocusRelationship} /> : null}
          {detail?.kind === "community" ? <CommunityDetail value={detail.value} report={detail.report} /> : null}
          {detail !== null && props.decision !== undefined && props.decision !== null ? <DecisionDetail value={props.decision} /> : null}
          {detail !== null ? <RawData value={detail} /> : null}
        </div>
      </ScrollArea>
    </section>
  )
}

function DecisionDetail({ value }: { value: ExplainabilityRecordView }): React.ReactElement {
  const { t } = useTranslation()
  const finalContext = value.finalContext === "included"
    ? t("Included")
    : value.finalContext === "excluded"
      ? t("Not included")
      : t("Unknown")
  return (
    <section className="mb-5 space-y-2 rounded-md border bg-primary/5 p-3" aria-label={t("Query decision")}>
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{t("Query decision")}</h3>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
        {value.score === undefined ? null : <DecisionMetric label={t("Retrieval score")} value={value.score.toFixed(4)} />}
        {value.rank === undefined ? null : <DecisionMetric label={t("Retrieval rank")} value={value.rank} />}
        <DecisionMetric label={t("Selection")} value={t(selectionLabel(value.selectionStatus))} />
        <DecisionMetric label={t("Final context")} value={finalContext} />
        {value.reason === undefined ? null : <DecisionMetric label={t("Reason")} value={value.reason.replaceAll("_", " ")} />}
      </dl>
    </section>
  )
}

function selectionLabel(status: ExplainabilityRecordView["selectionStatus"]): string {
  if (status === "pending") return "Retrieved"
  if (status === "selected") return "Selected"
  return "Excluded"
}

function DecisionMetric({ label, value }: { label: string; value: string | number }): React.ReactElement {
  return <div><dt className="text-muted-foreground">{label}</dt><dd className="mt-0.5 font-medium capitalize">{value}</dd></div>
}

function RawData({ value }: { value: GraphDetail }): React.ReactElement {
  const { t } = useTranslation()
  return <details className="rounded-md border bg-muted/20 p-3"><summary className="cursor-pointer text-xs font-semibold text-muted-foreground">{t("Developer · Raw JSON")}</summary><pre className="mt-3 overflow-x-auto whitespace-pre-wrap break-all text-[10px] leading-4">{JSON.stringify(value, null, 2)}</pre></details>
}

function Section({ title, children }: { title: string; children: React.ReactNode }): React.ReactElement {
  const { t } = useTranslation()
  return <section className="space-y-2"><h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{t(title)}</h3>{children}</section>
}

function IdBadges({ values }: { values: string[] }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="flex flex-wrap gap-1">{values.length === 0 ? <span className="text-sm text-muted-foreground">{t("None")}</span> : values.map((value) => <Badge key={value} variant="outline">{value}</Badge>)}</div>
}

function Metadata({ id, shortId }: { id: string; shortId?: string | null }): React.ReactElement {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  const copy = (): void => {
    void navigator.clipboard.writeText(id).then(() => setCopied(true)).catch(() => setCopied(false))
  }
  return (
    <details className="rounded-md border bg-muted/30 p-3 text-xs">
      <summary className="cursor-pointer font-medium text-muted-foreground">{t("Metadata")}</summary>
      <div className="mt-3 space-y-2">
        {shortId !== undefined && shortId !== null ? <p><span className="text-muted-foreground">{t("Short ID")}:</span> {shortId}</p> : null}
        <div className="flex items-center gap-2"><code className="min-w-0 flex-1 truncate">{id}</code><Button variant="ghost" size="icon" aria-label={t("Copy ID")} onClick={copy}>{copied ? <Check /> : <Copy />}</Button></div>
      </div>
    </details>
  )
}

function SourceIds({ values }: { values: string[] }): React.ReactElement {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? values : values.slice(0, 20)
  return (
    <details className="rounded-md border p-3">
      <summary className="cursor-pointer text-sm font-medium">{t("{{count}} source text unit", { count: values.length })}</summary>
      <div className="mt-3 space-y-2">
        <IdBadges values={visible} />
        {!showAll && values.length > visible.length ? <Button variant="ghost" size="sm" onClick={() => setShowAll(true)}>{t("Show {{count}} more", { count: values.length - visible.length })}</Button> : null}
      </div>
    </details>
  )
}

function EntityDetail({ value, onFocus }: { value: GraphEntityDetail; onFocus: (id: string) => void }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="space-y-5 pb-5">
      <div className="flex items-start justify-between gap-3 rounded-lg border bg-card p-4">
        <div className="flex min-w-0 gap-3"><div className="rounded-full bg-primary/10 p-2 text-primary"><Building2 className="size-5" /></div><div className="min-w-0"><h3 className="truncate font-semibold">{value.title}</h3><div className="mt-1 flex flex-wrap gap-1"><Badge variant="outline">{value.entity_type ?? t("Untyped")}</Badge><Badge variant="outline">{t("Degree {{value}}", { value: value.degree ?? "—" })}</Badge><Badge variant="outline">{t("Rank {{value}}", { value: value.rank ?? "—" })}</Badge></div></div></div>
        <Button size="sm" onClick={() => onFocus(value.id)}><Network /> {t("Focus neighborhood")}</Button>
      </div>
      <Section title="Description"><p className="whitespace-pre-wrap text-sm leading-6">{value.description ?? t("No description")}</p></Section>
      <Separator />
      <Section title="Communities"><IdBadges values={value.community_ids} /></Section>
      <Section title="Sources"><SourceIds values={value.text_unit_ids} /></Section>
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}

function RelationshipDetail({ value, onFocus }: { value: GraphRelationshipDetail; onFocus: (id: string) => void }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="space-y-5 pb-5">
      <div className="rounded-lg border bg-card p-4 text-center">
        <div className="font-semibold">{value.source}</div><ArrowDown className="mx-auto my-2 size-5 text-primary" /><div className="font-semibold">{value.target}</div>
        <div className="mt-3 flex justify-center gap-1"><Badge variant="outline">{t("Weight {{value}}", { value: value.weight ?? "—" })}</Badge><Badge variant="outline">{t("Rank {{value}}", { value: value.rank ?? "—" })}</Badge></div>
      </div>
      <Button className="w-full" onClick={() => onFocus(value.id)}><GitBranch /> {t("Focus relationship")}</Button>
      <Section title="Description"><p className="whitespace-pre-wrap text-sm leading-6">{value.description ?? t("No description")}</p></Section>
      <Section title="Sources"><SourceIds values={value.text_unit_ids} /></Section>
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}

function CommunityDetail({ value, report }: { value: GraphCommunity; report: GraphCommunityReportDetail | null }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="space-y-5 pb-5">
      <div className="flex gap-3 rounded-lg border bg-card p-4"><div className="rounded-full bg-primary/10 p-2 text-primary"><UsersRound className="size-5" /></div><div><h3 className="font-semibold">{value.title}</h3><div className="mt-1 flex flex-wrap gap-1"><Badge variant="outline">{t("Level {{value}}", { value: value.level })}</Badge><Badge variant="outline">{t("Short ID {{value}}", { value: value.short_id })}</Badge></div></div></div>
      <Section title="Summary"><p className="whitespace-pre-wrap text-sm leading-6">{value.report?.summary ?? t("No report summary")}</p></Section>
      <Section title="Hierarchy"><p className="text-sm"><span className="text-muted-foreground">{t("Parent")}:</span> {value.parent}</p><p className="text-sm"><span className="text-muted-foreground">{t("Children")}:</span> {value.children.join(", ") || t("None")}</p></Section>
      {report !== null ? <Section title="Report"><div className="mb-2 flex items-center gap-2"><GitBranch className="size-4 text-primary" /><span className="font-medium">{report.title}</span>{report.rank !== null ? <Badge variant="outline">{t("Rank {{value}}", { value: report.rank })}</Badge> : null}</div><SafeMarkdown>{report.full_content}</SafeMarkdown></Section> : null}
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}
