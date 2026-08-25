import { useEffect, useMemo, useState } from "react"
import { AlertCircle, Clock3, MessageSquareText, Sparkles } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityEnvelope, QueryResultState } from "@/api/types"
import { SafeMarkdown } from "@/components/content/safe-markdown"
import { GraphSourceEvidence } from "@/components/graph/graph-source-evidence"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { HoverCard, HoverCardContent, HoverCardTrigger } from "@/components/ui/hover-card"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Skeleton } from "@/components/ui/skeleton"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { useTextUnitEvidence } from "@/contexts/text-unit-evidence"
import { buildCitationEvidenceIndex, resolveCitationTarget, type CitationGroup, type CitationTarget, type GraphEmphasis } from "@/lib/citations"
import { buildQueryUsagePresentation, type QueryUsagePresentation } from "@/lib/query-usage-presentation"
import { detectRunContentMode } from "@/lib/timeline-presentation"

interface AnswerPanelProps {
  runId: string | null
  result: QueryResultState
  loading: boolean
  envelopes?: ExplainabilityEnvelope[]
  onCitationEmphasis?: (emphasis: GraphEmphasis) => void
}

export function AnswerPanel({ runId, result, loading, envelopes = [], onCitationEmphasis }: AnswerPanelProps): React.ReactElement {
  const { t } = useTranslation()
  const citationIndex = useMemo(() => buildCitationEvidenceIndex(envelopes), [envelopes])
  const showRawUsageCategories = detectRunContentMode(envelopes) === "debug"
  const [sourceViewer, setSourceViewer] = useState<{ group: CitationGroup; target: Extract<CitationTarget, { kind: "sources" }> } | null>(null)
  useEffect(() => setSourceViewer(null), [runId])
  const renderCitation = (group: CitationGroup): React.ReactNode => {
    const target = resolveCitationTarget(group, citationIndex)
    const title = `${group.dataset}\n${group.recordIds.join("\n")}${group.hasMore ? "\n+more" : ""}`
    const label = `${group.dataset} · ${group.recordIds.length}${group.hasMore ? "+" : ""}`
    if (target?.kind === "sources") {
      return <SourceCitation group={group} label={label} target={target} onOpen={() => setSourceViewer({ group, target })} />
    }
    if (target?.kind !== "graph" || onCitationEmphasis === undefined) {
      return <span className="mx-0.5 inline-flex items-center rounded-full border bg-muted/50 px-2 py-0.5 align-baseline text-[11px] font-medium text-muted-foreground" title={title}>{label}</span>
    }
    return <button type="button" className="mx-0.5 inline-flex items-center rounded-full border border-primary/40 bg-primary/10 px-2 py-0.5 align-baseline text-[11px] font-medium text-primary transition-colors hover:bg-primary/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={title} aria-label={t("answer.counts.emphasizeCountDatasetInGraph", { count: group.recordIds.length, dataset: group.dataset })} onClick={() => onCitationEmphasis({ entityIds: target.entityIds, relationshipIds: target.relationshipIds })}>{label}</button>
  }

  return (
    <section className="min-w-0 max-w-full overflow-x-hidden" aria-label={t("answer.title")}>
        <div className="min-w-0 max-w-full px-3 pb-3">
          {loading ? <><Skeleton className="mb-3 h-4 w-1/3" /><Skeleton className="h-20" /></> : null}
          {!loading && runId === null ? <AnswerState title={t("answer.labels.noResultYet")} detail={t("answer.messages.selectOrSubmitAQueryRun")} /> : null}
          {!loading && result.state === "waiting" && runId !== null ? <AnswerState title={t("answer.labels.queryIsRunning")} detail={t("answer.messages.theFinalBusinessResultWillAppearHereWhenTheRunCompletes")} /> : null}
          {!loading && result.state === "failed" ? <AnswerState title={t("answer.labels.queryDidNotComplete")} detail={t("answer.messages.explainabilityFailureHint")} tone="error" /> : null}
          {!loading && result.state === "gone" ? <AnswerState title={t("answer.labels.resultNoLongerRetained")} detail={t("answer.messages.retainedResultUnavailable")} /> : null}
          {!loading && result.state === "missing" ? <AnswerState title={t("answer.labels.resultUnavailable")} detail={t("answer.messages.theQueryRunDoesNotExistInThisStoreNamespace")} /> : null}
          {!loading && result.state === "ready" ? (
            <div className="grid gap-4">
              <article className="min-w-0 max-w-full overflow-x-hidden"><SafeMarkdown renderCitation={renderCitation}>{result.result.response}</SafeMarkdown></article>
              <UsageFooter elapsedMs={result.result.elapsed_ms} presentation={buildQueryUsagePresentation(result.result.usage, envelopes)} showRawCategories={showRawUsageCategories} />
            </div>
          ) : null}
        </div>
        {sourceViewer === null ? null : <SourceEvidenceViewer viewer={sourceViewer} onClose={() => setSourceViewer(null)} />}
    </section>
  )
}

function SourceEvidenceViewer({ viewer, onClose }: { viewer: { group: CitationGroup; target: Extract<CitationTarget, { kind: "sources" }> }; onClose: () => void }): React.ReactElement {
  const { t } = useTranslation()
  const evidence = useTextUnitEvidence()
  const loadingSourceIds = new Set(viewer.target.textUnitIds.filter((id) => {
    const status = evidence.status(id)
    return status === "idle" || status === "loading"
  }))
  return (
    <Sheet open onOpenChange={(open) => { if (!open) onClose() }}>
      <SheetContent className="min-w-0 overflow-y-auto">
        <SheetHeader>
          <SheetTitle>{t("answer.sources.sourceEvidenceCount", { count: viewer.target.textUnitIds.length })}</SheetTitle>
          <SheetDescription>{t("answer.sources.sourceEvidenceDescription")}</SheetDescription>
        </SheetHeader>
        {viewer.group.hasMore ? <p className="rounded-md border bg-muted/20 p-2 text-xs text-muted-foreground">{t("answer.sources.additionalSourcesOmitted")}</p> : null}
        {viewer.target.unresolvedCount === 0 ? null : <p className="rounded-md border border-warning/30 bg-warning/5 p-2 text-xs text-muted-foreground">{t("answer.sources.unresolvedCount", { count: viewer.target.unresolvedCount })}</p>}
        <GraphSourceEvidence sourceIds={viewer.target.textUnitIds} sources={viewer.target.textUnitIds.flatMap((id) => { const reference = evidence.refs.get(id); return reference === undefined ? [] : [reference] })} loadingSourceIds={loadingSourceIds} />
      </SheetContent>
    </Sheet>
  )
}

function SourceCitation({ group, label, target, onOpen }: { group: CitationGroup; label: string; target: Extract<CitationTarget, { kind: "sources" }>; onOpen: () => void }): React.ReactElement {
  const { t } = useTranslation()
  const evidence = useTextUnitEvidence()
  const [hoverPreviewOpen, setHoverPreviewOpen] = useState(false)
  const [focusPreviewOpen, setFocusPreviewOpen] = useState(false)
  const load = (): void => evidence.resolve(target.textUnitIds)
  return (
    <HoverCard open={hoverPreviewOpen || focusPreviewOpen} onOpenChange={(open) => { setHoverPreviewOpen(open); if (open) load() }}>
      <HoverCardTrigger asChild>
        <span className="contents">
          <button type="button" className="mx-0.5 inline-flex items-center rounded-full border border-primary/40 bg-primary/10 px-2 py-0.5 align-baseline text-[11px] font-medium text-primary transition-colors hover:bg-primary/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-label={t("answer.sources.viewEvidence", { count: group.recordIds.length })} onPointerEnter={() => { load(); setFocusPreviewOpen(true) }} onPointerLeave={() => setFocusPreviewOpen(false)} onFocus={() => { load(); setFocusPreviewOpen(true) }} onBlur={() => setFocusPreviewOpen(false)} onClick={() => { load(); setFocusPreviewOpen(false); setHoverPreviewOpen(false); onOpen() }}>{label}</button>
        </span>
      </HoverCardTrigger>
      <HoverCardContent className="w-96 max-w-[calc(100vw-2rem)] space-y-2" align="start">
        <p className="text-xs font-semibold">{t("answer.sources.sourceEvidenceCount", { count: target.textUnitIds.length })}</p>
        {target.textUnitIds.map((id) => {
          const reference = evidence.refs.get(id)
          if (reference === undefined) return <p key={id} className="text-xs text-muted-foreground">{t(evidence.status(id) === "unavailable" ? "explainability.sources.previewUnavailable" : "explainability.sources.loadingPreview")}</p>
          return <div key={id} className="min-w-0 border-t pt-2 first:border-t-0 first:pt-0"><p className="text-xs font-medium">{t("graph.sources.textUnitNumber", { shortId: reference.short_id })}</p><p className="mt-1 line-clamp-3 whitespace-normal break-words text-xs leading-5 text-muted-foreground">{reference.preview}</p></div>
        })}
        {target.unresolvedCount === 0 ? null : <p className="border-t pt-2 text-xs text-muted-foreground">{t("answer.sources.unresolvedCount", { count: target.unresolvedCount })}</p>}
        {group.hasMore ? <p className="border-t pt-2 text-xs text-muted-foreground">{t("answer.sources.additionalSourcesOmitted")}</p> : null}
      </HoverCardContent>
    </HoverCard>
  )
}

function UsageFooter({ elapsedMs, presentation, showRawCategories }: { elapsedMs: number; presentation: QueryUsagePresentation; showRawCategories: boolean }): React.ReactElement {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const showModel = presentation.rows.some((row) => row.model !== undefined)
  const operationSummary = presentation.basicOperations === undefined
    ? t("answer.usage.modelOperations", { count: presentation.totalOperations })
    : t("answer.usage.basicSummary", {
        embedding: presentation.basicOperations.embedding,
        generation: presentation.basicOperations.generation,
      })
  return (
    <aside className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline"><Clock3 /> {formatDuration(elapsedMs)}</Badge>
        <Badge variant="outline" title={t("answer.usage.modelOperationsHelp")}><Sparkles /> {operationSummary}</Badge>
      </div>
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger asChild><Button variant="ghost" size="sm" aria-expanded={open}>{t("answer.usage.usageDetails")}</Button></CollapsibleTrigger>
        <CollapsibleContent className="mt-1 max-w-full overflow-x-auto rounded-md border">
          <Table>
            <TableHeader><TableRow><TableHead>{t("answer.usage.stage")}</TableHead>{showModel ? <TableHead>{t("answer.usage.model")}</TableHead> : null}<TableHead>{t("answer.usage.operation")}</TableHead><TableHead>{t("answer.usage.operations")}</TableHead><TableHead>{t("answer.usage.inputTokens")}</TableHead><TableHead>{t("answer.usage.outputTokens")}</TableHead></TableRow></TableHeader>
            <TableBody>{presentation.rows.map((row) => <TableRow key={row.rawCategory || "total"}><TableCell>{row.stageKey === undefined ? row.stageFallback : t(row.stageKey)}{showRawCategories && row.rawCategory.length > 0 ? <code className="mt-0.5 block text-[10px] text-muted-foreground">{row.rawCategory}</code> : null}</TableCell>{showModel ? <TableCell>{row.model ?? "—"}</TableCell> : null}<TableCell>{t(row.operationKey)}</TableCell><TableCell>{row.calls.toLocaleString()}</TableCell><TableCell>{row.inputTokens.toLocaleString()}</TableCell><TableCell>{row.outputApplicable ? row.outputTokens.toLocaleString() : "—"}</TableCell></TableRow>)}</TableBody>
          </Table>
        </CollapsibleContent>
      </Collapsible>
    </aside>
  )
}

function formatDuration(elapsedMs: number): string {
  if (elapsedMs < 1_000) return `${elapsedMs.toLocaleString()} ms`
  return `${(elapsedMs / 1_000).toLocaleString(undefined, { maximumFractionDigits: 1 })} s`
}

function AnswerState({ title, detail, tone = "neutral" }: { title: string; detail: string; tone?: "neutral" | "error" }): React.ReactElement {
  return <div className="flex flex-col items-start py-3">{tone === "error" ? <AlertCircle className="mb-2 size-5 text-destructive" /> : <MessageSquareText className="mb-2 size-5 text-muted-foreground/50" />}<p className="text-sm font-medium">{title}</p><p className="mt-1 max-w-xl text-xs text-muted-foreground">{detail}</p></div>
}
