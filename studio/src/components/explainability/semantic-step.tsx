import { useState } from "react"
import type { TFunction } from "i18next"
import { Braces, Check, Circle, DatabaseZap, GitBranch, Sparkles, X } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityContextSection, ExplainabilityEnvelope } from "@/api/types"
import { CapturedContentViewer } from "@/components/explainability/captured-content-viewer"
import { TechnicalDetails } from "@/components/explainability/technical-details"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { ExplainabilityRecordView, LocalSemanticStep } from "@/lib/semantic-timeline"
import { contextSectionTitle, semanticStepTitle } from "@/i18n/presentation"
import type { StudioTranslationKey } from "@/i18n/types"

interface SemanticStepCardProps {
  step: LocalSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  onInspectCandidate: (candidate: ExplainabilityRecordView) => void
}

const RECORD_LABELS: Readonly<Record<string, StudioTranslationKey>> = {
  relationship: "explainability.labels.relationships",
  community_report: "explainability.labels.reports",
  text_unit: "graph.labels.sources",
  covariate: "explainability.labels.claims",
}

const RECORD_TYPE_LABELS: Readonly<Record<string, StudioTranslationKey>> = {
  entity: "graph.status.entity",
  relationship: "graph.status.relationship",
  community_report: "explainability.labels.reports",
  text_unit: "explainability.labels.textUnits",
  covariate: "explainability.labels.covariates",
}

export function SemanticStepCard({ step, onFocusGraph, onInspectCandidate }: SemanticStepCardProps): React.ReactElement {
  const { t } = useTranslation()
  const focusEnvelope = step.focusEnvelope
  const title = semanticStepTitle(t, step.kind)
  return (
    <article className="rounded-md border bg-card/70 p-3" aria-label={title}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 gap-2">
          <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><StepIcon kind={step.kind} /></span>
          <div className="min-w-0"><h3 className="text-sm font-semibold">{title}</h3><StepSummary step={step} /></div>
        </div>
        {focusEnvelope === null ? null : <Button variant="outline" size="sm" onClick={() => onFocusGraph(focusEnvelope)}>{t("runs.actions.focusInGraph")}</Button>}
      </div>
      <StepContent step={step} onInspectCandidate={onInspectCandidate} />
      <TechnicalDetails rawEvents={step.rawEvents} onFocusGraph={onFocusGraph} />
    </article>
  )
}

function StepIcon({ kind }: { kind: LocalSemanticStep["kind"] }): React.ReactElement {
  const className = "size-4"
  if (kind === "entity-mapping") return <DatabaseZap className={className} />
  if (kind === "graph-expansion") return <GitBranch className={className} />
  if (kind === "context-assembly") return <Braces className={className} />
  return <Sparkles className={className} />
}

function StepSummary({ step }: { step: LocalSemanticStep }): React.ReactElement {
  const { t } = useTranslation()
  if (step.kind === "entity-mapping") {
    const summary = step.summary
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countRetrieved", { count: summary.retrievedCount })} · {t("explainability.counts.countSelected", { count: summary.selectedCount })} · {t("explainability.counts.countExcluded", { count: summary.excludedCount })}{summary.pendingCount === 0 ? "" : ` · ${t("explainability.counts.countPending", { count: summary.pendingCount })}`}</p>
  }
  if (step.kind === "graph-expansion") {
    const total = Object.values(step.summary.selectedCounts).reduce((sum, count) => sum + count, 0)
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countContextRecordSelectedThroughGraphExpansion", { count: total })}</p>
  }
  if (step.kind === "context-assembly") {
    const { tokensUsed, totalTokenBudget } = step.summary
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countSection", { count: step.summary.sections.length })}{tokensUsed === undefined ? "" : ` · ${tokensUsed.toLocaleString()}${totalTokenBudget === undefined ? "" : ` / ${totalTokenBudget.toLocaleString()}`} ${t("explainability.status.tokens")}`}</p>
  }
  return <p className="mt-1 text-xs text-muted-foreground">{t("answer.counts.countCall", { count: step.summary.calls })} · {t("answer.counts.countInput", { count: step.summary.inputTokens })} · {t("answer.counts.countOutput", { count: step.summary.outputTokens })}</p>
}

function StepContent({ step, onInspectCandidate }: { step: LocalSemanticStep; onInspectCandidate: (candidate: ExplainabilityRecordView) => void }): React.ReactNode {
  const { t } = useTranslation()
  if (step.kind === "entity-mapping") {
    const metadata = [
      step.summary.model,
      step.summary.elapsedMs === undefined ? undefined : `${step.summary.elapsedMs} ms`,
      step.summary.promptTokens === undefined ? undefined : t("explainability.counts.countEmbeddingToken", { count: step.summary.promptTokens }),
      step.summary.dimensions === undefined ? undefined : t("explainability.counts.countDimension", { count: step.summary.dimensions }),
    ].filter((value): value is string => value !== undefined)
    return <div className="mt-3 space-y-3">{metadata.length === 0 ? null : <p className="text-[11px] text-muted-foreground">{metadata.join(" · ")}</p>}<DecisionRecordList records={step.summary.candidates} onInspectCandidate={onInspectCandidate} /></div>
  }
  if (step.kind === "graph-expansion") {
    const counts = Object.entries(step.summary.selectedCounts).filter(([, count]) => count > 0)
    return <div className="mt-3 space-y-3">{counts.length === 0 ? <p className="text-xs text-muted-foreground">{t("explainability.messages.noExpansionRecordsWereSelected")}</p> : <dl className="grid grid-cols-2 gap-2 text-xs">{counts.map(([type, count]) => <div key={type} className="flex justify-between rounded border bg-muted/20 px-2 py-1.5"><dt>{recordCollectionLabel(t, type)}</dt><dd className="font-mono">{count}</dd></div>)}</dl>}<DecisionRecordList records={step.summary.records} onInspectCandidate={onInspectCandidate} /></div>
  }
  if (step.kind === "context-assembly") {
    return <div className="mt-3 min-w-0 space-y-2">{step.summary.sections.map((section) => <ContextSectionRow key={`${section.section}:${section.name ?? ""}`} section={section} />)}<CapturedContentViewer buttonLabel="explainability.actions.viewLlmContext" title={t("explainability.labels.llmContext")} content={step.summary.exactContext} unavailableMessage="explainability.messages.llmContextNotCaptured" testId="exact-llm-context" exactTabLabel="explainability.labels.exactInput" copyLabel="explainability.actions.copyExactLlmContext" description="explainability.messages.capturedInputPreviewNotice" /></div>
  }
  const summary = step.summary
  return <dl className="mt-3 grid grid-cols-2 gap-2 text-xs"><Metric label="answer.labels.calls" value={summary.calls} /><Metric label="explainability.labels.inputTokens" value={summary.inputTokens.toLocaleString()} /><Metric label="explainability.labels.outputTokens" value={summary.outputTokens.toLocaleString()} /><Metric label="explainability.labels.latency" value={`${summary.elapsedMs.toLocaleString()} ms`} />{summary.model === undefined ? null : <Metric label="explainability.labels.model" value={summary.model} />}</dl>
}

function Metric({ label, value }: { label: StudioTranslationKey; value: string | number }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="rounded border bg-muted/20 px-2 py-1.5"><dt className="text-muted-foreground">{t(label)}</dt><dd className="mt-0.5 font-medium">{value}</dd></div>
}

function DecisionRecordList({ records, onInspectCandidate }: { records: ExplainabilityRecordView[]; onInspectCandidate: (candidate: ExplainabilityRecordView) => void }): React.ReactElement | null {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  if (records.length === 0) return null
  const visible = expanded ? records : records.slice(0, 20)
  return (
    <div className="space-y-2">
      <div className="divide-y rounded-md border bg-background/50">
        {visible.map((record) => <DecisionRecordRow key={`${record.recordType}:${record.stableId}`} record={record} onInspectCandidate={onInspectCandidate} />)}
      </div>
      {!expanded && visible.length < records.length ? <Button variant="ghost" size="sm" onClick={() => setExpanded(true)}>{t("explainability.counts.showAllCountRecords", { count: records.length })}</Button> : null}
    </div>
  )
}

function DecisionRecordRow({ record, onInspectCandidate }: { record: ExplainabilityRecordView; onInspectCandidate: (candidate: ExplainabilityRecordView) => void }): React.ReactElement {
  const { t } = useTranslation()
  const inspectable = (record.recordType === "entity" || record.recordType === "relationship") && record.stableId.length > 0
  const label = record.title ?? record.shortId ?? record.stableId
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-x-2 gap-y-1 overflow-hidden px-2 py-2 text-xs">
      <span className="row-span-2 pt-0.5"><DecisionIcon record={record} /></span>
      <div className="min-w-0">
        {inspectable ? <button type="button" className="block max-w-full truncate text-left font-medium hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={label} aria-label={t("explainability.actions.inspectTypeLabel", { type: recordTypeLabel(t, record.recordType), label })} onClick={() => onInspectCandidate(record)}>{label}</button> : <span className="block truncate font-medium" title={label}>{label}</span>}
      </div>
      <Badge variant="outline" className="max-w-36 shrink-0 truncate" title={t(decisionLabel(record))}>{t(decisionLabel(record))}</Badge>
      <div className="col-span-2 col-start-2 flex min-w-0 items-center gap-2 text-[10px] text-muted-foreground">
        <span className="min-w-0 flex-1 truncate font-mono" title={record.stableId}>{record.shortId === undefined ? compactStableId(record.stableId) : `${record.shortId} · ${compactStableId(record.stableId)}`}</span>
        {record.score === undefined ? null : <span className="shrink-0 font-mono text-[11px]" aria-label={t("explainability.labels.scoreValue", { value: record.score.toFixed(4) })}>{record.score.toFixed(4)}</span>}
      </div>
    </div>
  )
}

function compactStableId(id: string): string {
  if (id.length <= 20) return id
  return `${id.slice(0, 8)}…${id.slice(-8)}`
}

function ContextSectionRow({ section }: { section: ExplainabilityContextSection }): React.ReactElement {
  const { t } = useTranslation()
  const title = contextSectionTitle(t, section.section, section.name)
  const emptySection = section.selected_count === 0 && section.tokens_used > 0
  const tokenExplanation = emptySection ? t("explainability.messages.emptySectionTokenNotice") : undefined
  return (
    <div className="min-w-0 overflow-hidden rounded border bg-muted/20 px-2 py-2 text-xs">
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
        <span className="truncate font-medium capitalize" title={title}>{title}</span>
        <span className="shrink-0">{t("explainability.labels.selectedCandidatesIncluded", { selected: section.selected_count, candidates: section.candidate_count })}</span>
      </div>
      <p className="mt-1 min-w-0 break-words text-[11px] text-muted-foreground" title={tokenExplanation}>
        {section.tokens_used.toLocaleString()} / {section.token_budget.toLocaleString()} {t("explainability.status.tokens")}{emptySection ? ` · ${t("explainability.status.emptySection")}` : ""}{section.truncated ? ` · ${t("explainability.status.truncated")}` : ""}
      </p>
    </div>
  )
}

function DecisionIcon({ record }: { record: ExplainabilityRecordView }): React.ReactElement {
  const { t } = useTranslation()
  if (record.selectionStatus === "pending") return <Circle className="size-3.5 shrink-0 text-muted-foreground" aria-label={t("explainability.labels.retrievedSelectionPending")} />
  if (record.selectionStatus === "excluded") return <X className="size-3.5 shrink-0 text-muted-foreground" aria-label={t("graph.labels.excluded")} />
  if (record.finalContext === "included") return <Check className="size-3.5 shrink-0 text-success" aria-label={t("explainability.labels.includedInFinalContext")} />
  return <Circle className="size-3.5 shrink-0 text-muted-foreground" aria-label={t(record.finalContext === "excluded" ? "explainability.labels.notInFinalContext" : "explainability.labels.finalContextUnknown")} />
}

function decisionLabel(record: ExplainabilityRecordView): StudioTranslationKey {
  if (record.selectionStatus === "pending") return "graph.labels.retrieved"
  if (record.selectionStatus === "excluded") return "graph.labels.excluded"
  if (record.finalContext === "included") return "graph.labels.included"
  if (record.finalContext === "excluded") return "explainability.labels.notInFinalContext"
  return "explainability.actions.selectedContextUnknown"
}

function recordTypeLabel(t: TFunction, recordType: string): string {
  const key = RECORD_TYPE_LABELS[recordType]
  return key === undefined ? recordType.replaceAll("_", " ") : t(key)
}

function recordCollectionLabel(t: TFunction, recordType: string): string {
  const key = RECORD_LABELS[recordType]
  return key === undefined ? recordType.replaceAll("_", " ") : t(key)
}
