import { useState } from "react"
import { ArrowDown, ArrowLeft, Building2, Check, Copy, GitBranch, Network, UsersRound, X } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { GraphCommunity, GraphCommunityReportDetail, GraphEntityDetail, GraphRelationshipDetail } from "@/api/types"
import { SafeMarkdown } from "@/components/content/safe-markdown"
import { GraphReferenceCard, UnresolvedGraphReference } from "@/components/graph/graph-reference-card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import type { ExplainabilityRecordView } from "@/lib/semantic-timeline"
import type { StudioTranslationKey } from "@/i18n/types"

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
  canGoBack: boolean
  onBack: () => void
  onOpenEntity: (id: string) => void
  onOpenCommunity: (id: string) => void
  onFocusEntity: (id: string) => void
  onFocusRelationship: (id: string) => void
}

export function GraphInspector(props: GraphInspectorProps): React.ReactElement {
  const { t } = useTranslation()
  const { detail } = props
  const title = detail === null
    ? t("graph.labels.graphItem")
    : detail.kind === "relationship"
      ? t("graph.labels.relationship")
      : detail.value.title
  return (
    <section className="flex size-full min-h-0 flex-col" aria-label={t("graph.labels.graphInspector")} tabIndex={-1} onKeyDown={(event) => { if (event.key === "Escape") props.onClear() }}>
      <header className="flex h-11 shrink-0 items-center justify-between border-b px-3">
        <div className="flex min-w-0 items-center gap-1">
          {props.canGoBack ? <Button variant="ghost" size="icon" className="size-8 shrink-0" aria-label={t("graph.navigation.back")} onClick={props.onBack}><ArrowLeft /></Button> : null}
          <div className="min-w-0"><h2 className="truncate text-sm font-semibold">{props.loading ? t("graph.actions.loadingGraphDetail") : title}</h2><p className="text-[10px] text-muted-foreground">{detail === null ? t("graph.actions.inspector") : t(graphKindKey(detail.kind))}</p></div>
        </div>
        {detail !== null || props.loading || props.error ? <Button variant="ghost" size="icon" className="size-8 shrink-0" aria-label={t("graph.actions.clearGraphSelection")} onClick={props.onClear}><X /></Button> : null}
      </header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="p-3">
          {detail === null && !props.loading && !props.error ? <div className="flex min-h-64 flex-col items-center justify-center px-5 text-center"><Network className="mb-3 size-8 text-muted-foreground/40" /><p className="text-sm font-medium">{t("graph.actions.selectAGraphObject")}</p><p className="mt-1 text-xs leading-5 text-muted-foreground">{t("graph.messages.selectANodeRelationshipOrCommunityToInspectIt")}</p></div> : null}
          {props.loading ? <p className="py-10 text-center text-sm text-muted-foreground">{t("graph.messages.loadingStructuredDetail")}</p> : null}
          {props.error ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-red-300">{t("graph.messages.graphDetailIsUnavailable")}</p> : null}
          {detail?.kind === "entity" ? <EntityDetail value={detail.value} onFocus={props.onFocusEntity} onOpenCommunity={props.onOpenCommunity} /> : null}
          {detail?.kind === "relationship" ? <RelationshipDetail value={detail.value} onFocus={props.onFocusRelationship} onOpenEntity={props.onOpenEntity} /> : null}
          {detail?.kind === "community" ? <CommunityDetail value={detail.value} report={detail.report} onOpenCommunity={props.onOpenCommunity} /> : null}
          {detail !== null && props.decision !== undefined && props.decision !== null ? <DecisionDetail value={props.decision} /> : null}
          {detail !== null ? <RawData value={detail} /> : null}
        </div>
      </ScrollArea>
    </section>
  )
}

function graphKindKey(kind: GraphDetail["kind"]): "graph.status.entity" | "graph.status.relationship" | "graph.status.community" {
  if (kind === "entity") return "graph.status.entity"
  if (kind === "relationship") return "graph.status.relationship"
  return "graph.status.community"
}

function DecisionDetail({ value }: { value: ExplainabilityRecordView }): React.ReactElement {
  const { t } = useTranslation()
  const finalContext = value.finalContext === "included"
    ? t("graph.labels.included")
    : value.finalContext === "excluded"
      ? t("graph.labels.notIncluded")
      : t("graph.labels.unknown")
  return (
    <section className="mb-5 space-y-2 rounded-md border bg-primary/5 p-3" aria-label={t("graph.labels.queryDecision")}>
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{t("graph.labels.queryDecision")}</h3>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
        {value.score === undefined ? null : <DecisionMetric label={t("graph.labels.retrievalScore")} value={value.score.toFixed(4)} />}
        {value.rank === undefined ? null : <DecisionMetric label={t("graph.labels.retrievalRank")} value={value.rank} />}
        <DecisionMetric label={t("graph.actions.selection")} value={t(selectionLabel(value.selectionStatus))} />
        <DecisionMetric label={t("graph.labels.finalContext")} value={finalContext} />
        {value.reason === undefined ? null : <DecisionMetric label={t("explainability.labels.reason")} value={value.reason.replaceAll("_", " ")} />}
      </dl>
    </section>
  )
}

function selectionLabel(status: ExplainabilityRecordView["selectionStatus"]): StudioTranslationKey {
  if (status === "pending") return "graph.labels.retrieved"
  if (status === "selected") return "explainability.actions.selected"
  return "graph.labels.excluded"
}

function DecisionMetric({ label, value }: { label: string; value: string | number }): React.ReactElement {
  return <div><dt className="text-muted-foreground">{label}</dt><dd className="mt-0.5 font-medium capitalize">{value}</dd></div>
}

function RawData({ value }: { value: GraphDetail }): React.ReactElement {
  const { t } = useTranslation()
  return <details className="rounded-md border bg-muted/20 p-3"><summary className="cursor-pointer text-xs font-semibold text-muted-foreground">{t("graph.labels.developerRawJson")}</summary><pre className="mt-3 overflow-x-auto whitespace-pre-wrap break-all text-[10px] leading-4">{JSON.stringify(value, null, 2)}</pre></details>
}

function Section({ title, children }: { title: StudioTranslationKey; children: React.ReactNode }): React.ReactElement {
  const { t } = useTranslation()
  return <section className="space-y-2"><h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{t(title)}</h3>{children}</section>
}

function IdBadges({ values }: { values: string[] }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="flex flex-wrap gap-1">{values.length === 0 ? <span className="text-sm text-muted-foreground">{t("graph.labels.none")}</span> : values.map((value) => <Badge key={value} variant="outline">{value}</Badge>)}</div>
}

function Metadata({ id, shortId }: { id: string; shortId?: string | null }): React.ReactElement {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  const copy = (): void => {
    void navigator.clipboard.writeText(id).then(() => setCopied(true)).catch(() => setCopied(false))
  }
  return (
    <details className="rounded-md border bg-muted/30 p-3 text-xs">
      <summary className="cursor-pointer font-medium text-muted-foreground">{t("graph.labels.metadata")}</summary>
      <div className="mt-3 space-y-2">
        {shortId !== undefined && shortId !== null ? <p><span className="text-muted-foreground">{t("graph.labels.shortId")}:</span> {shortId}</p> : null}
        <div className="flex items-center gap-2"><code className="min-w-0 flex-1 truncate">{id}</code><Button variant="ghost" size="icon" aria-label={t("graph.actions.copyId")} onClick={copy}>{copied ? <Check /> : <Copy />}</Button></div>
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
      <summary className="cursor-pointer text-sm font-medium">{t("graph.counts.countSourceTextUnit", { count: values.length })}</summary>
      <div className="mt-3 space-y-2">
        <IdBadges values={visible} />
        {!showAll && values.length > visible.length ? <Button variant="ghost" size="sm" onClick={() => setShowAll(true)}>{t("graph.counts.showCountMore", { count: values.length - visible.length })}</Button> : null}
      </div>
    </details>
  )
}

function EntityDetail({ value, onFocus, onOpenCommunity }: { value: GraphEntityDetail; onFocus: (id: string) => void; onOpenCommunity: (id: string) => void }): React.ReactElement {
  const { t } = useTranslation()
  const resolvedIds = new Set(value.communities.map((community) => community.short_id))
  const unresolvedIds = [...new Set(value.community_ids)].filter((id) => !resolvedIds.has(id))
  return (
    <div className="space-y-5 pb-5">
      <div className="flex items-start justify-between gap-3 rounded-lg border bg-card p-4">
        <div className="flex min-w-0 gap-3"><div className="rounded-full bg-primary/10 p-2 text-primary"><Building2 className="size-5" /></div><div className="min-w-0"><h3 className="truncate font-semibold">{value.title}</h3><div className="mt-1 flex flex-wrap gap-1"><Badge variant="outline">{value.entity_type ?? t("graph.labels.untyped")}</Badge><Badge variant="outline">{t("graph.labels.degreeValue", { value: value.degree ?? "—" })}</Badge><Badge variant="outline">{t("graph.labels.rankValue", { value: value.rank ?? "—" })}</Badge></div></div></div>
        <Button size="sm" onClick={() => onFocus(value.id)}><Network /> {t("graph.actions.focusNeighborhood")}</Button>
      </div>
      <Section title="graph.labels.description"><p className="whitespace-pre-wrap text-sm leading-6">{value.description ?? t("graph.labels.noDescription")}</p></Section>
      <Separator />
      <Section title="graph.navigation.relatedCommunities">
        <div className="space-y-2">
          {value.communities.map((community) => <GraphReferenceCard key={community.id} reference={{ kind: "community", value: community }} onOpen={onOpenCommunity} />)}
          {unresolvedIds.map((id) => <UnresolvedGraphReference key={id} value={id} label={t("graph.navigation.unresolvedCommunity", { id })} />)}
          {value.communities.length === 0 && unresolvedIds.length === 0 ? <span className="text-sm text-muted-foreground">{t("graph.labels.none")}</span> : null}
        </div>
      </Section>
      <Section title="graph.labels.sources"><SourceIds values={value.text_unit_ids} /></Section>
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}

function RelationshipDetail({ value, onFocus, onOpenEntity }: { value: GraphRelationshipDetail; onFocus: (id: string) => void; onOpenEntity: (id: string) => void }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="space-y-5 pb-5">
      <div className="rounded-lg border bg-card p-4">
        <p className="mb-2 text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">{t("graph.navigation.sourceEntity")}</p>
        {value.source_entity === null ? <UnresolvedGraphReference value={value.source} label={t("graph.navigation.unresolvedEntity")} /> : <GraphReferenceCard reference={{ kind: "entity", value: value.source_entity }} onOpen={onOpenEntity} />}
        <ArrowDown className="mx-auto my-2 size-5 text-primary" />
        <p className="mb-2 text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">{t("graph.navigation.targetEntity")}</p>
        {value.target_entity === null ? <UnresolvedGraphReference value={value.target} label={t("graph.navigation.unresolvedEntity")} /> : <GraphReferenceCard reference={{ kind: "entity", value: value.target_entity }} onOpen={onOpenEntity} />}
        <div className="mt-3 flex justify-center gap-1"><Badge variant="outline">{t("graph.labels.weightValue", { value: value.weight ?? "—" })}</Badge><Badge variant="outline">{t("graph.labels.rankValue", { value: value.rank ?? "—" })}</Badge></div>
      </div>
      <Button className="w-full" onClick={() => onFocus(value.id)}><GitBranch /> {t("graph.actions.focusRelationship")}</Button>
      <Section title="graph.labels.description"><p className="whitespace-pre-wrap text-sm leading-6">{value.description ?? t("graph.labels.noDescription")}</p></Section>
      <Section title="graph.labels.sources"><SourceIds values={value.text_unit_ids} /></Section>
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}

const INITIAL_CHILD_COUNT = 10

function CommunityDetail({ value, report, onOpenCommunity }: { value: GraphCommunity; report: GraphCommunityReportDetail | null; onOpenCommunity: (id: string) => void }): React.ReactElement {
  const { t } = useTranslation()
  const [showAllChildren, setShowAllChildren] = useState(false)
  const childReferences = new Map(value.child_communities.map((community) => [community.short_id, community]))
  const childItems = [...new Set(value.children)].map((id) => ({ id, reference: childReferences.get(String(id)) ?? null }))
  const visibleChildren = showAllChildren ? childItems : childItems.slice(0, INITIAL_CHILD_COUNT)
  return (
    <div className="space-y-5 pb-5">
      <div className="flex gap-3 rounded-lg border bg-card p-4"><div className="rounded-full bg-primary/10 p-2 text-primary"><UsersRound className="size-5" /></div><div><h3 className="font-semibold">{value.title}</h3><div className="mt-1 flex flex-wrap gap-1"><Badge variant="outline">{t("graph.labels.levelValue", { value: value.level })}</Badge><Badge variant="outline">{t("graph.labels.shortIdValue", { value: value.short_id })}</Badge></div></div></div>
      <Section title="graph.labels.summary"><p className="whitespace-pre-wrap text-sm leading-6">{value.report?.summary ?? t("graph.labels.noReportSummary")}</p></Section>
      <Section title="graph.labels.hierarchy">
        <div className="space-y-3">
          <div className="space-y-2"><p className="text-[11px] font-medium text-muted-foreground">{t("graph.navigation.parentCommunity")}</p>{value.parent < 0 ? <p className="text-sm text-muted-foreground">{t("graph.navigation.rootCommunity")}</p> : value.parent_community === null ? <UnresolvedGraphReference value={String(value.parent)} label={t("graph.navigation.unresolvedCommunity", { id: value.parent })} /> : <GraphReferenceCard reference={{ kind: "community", value: value.parent_community }} onOpen={onOpenCommunity} />}</div>
          <div className="space-y-2"><p className="text-[11px] font-medium text-muted-foreground">{t("graph.navigation.childCommunities")}</p>{visibleChildren.map((child) => child.reference === null ? <UnresolvedGraphReference key={child.id} value={String(child.id)} label={t("graph.navigation.unresolvedCommunity", { id: child.id })} /> : <GraphReferenceCard key={child.reference.id} reference={{ kind: "community", value: child.reference }} onOpen={onOpenCommunity} />)}{value.children.length === 0 ? <p className="text-sm text-muted-foreground">{t("graph.navigation.noChildCommunities")}</p> : null}{childItems.length > INITIAL_CHILD_COUNT ? <Button variant="ghost" size="sm" onClick={() => setShowAllChildren((current) => !current)}>{t(showAllChildren ? "graph.navigation.showFewerChildren" : "graph.navigation.showAllChildren")}</Button> : null}</div>
        </div>
      </Section>
      {report !== null ? <Section title="graph.labels.report"><div className="mb-2 flex items-center gap-2"><GitBranch className="size-4 text-primary" /><span className="font-medium">{report.title}</span>{report.rank !== null ? <Badge variant="outline">{t("graph.labels.rankValue", { value: report.rank })}</Badge> : null}</div><SafeMarkdown>{report.full_content}</SafeMarkdown></Section> : null}
      <Metadata id={value.id} shortId={value.short_id} />
    </div>
  )
}
