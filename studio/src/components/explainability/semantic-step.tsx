import { useState } from "react"
import { Braces, Check, Circle, DatabaseZap, GitBranch, Sparkles, X } from "lucide-react"

import type { ExplainabilityEnvelope } from "@/api/types"
import { TimelineEvent } from "@/components/explainability/timeline-event"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import type { ExplainabilityRecordView, SemanticStep } from "@/lib/semantic-timeline"

interface SemanticStepCardProps {
  step: SemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  onInspectCandidate: (candidate: ExplainabilityRecordView) => void
}

const RECORD_LABELS: Record<string, string> = {
  relationship: "Relationships",
  community_report: "Reports",
  text_unit: "Sources",
  covariate: "Claims",
}

export function SemanticStepCard({ step, onFocusGraph, onInspectCandidate }: SemanticStepCardProps): React.ReactElement {
  const focusEnvelope = step.focusEnvelope
  return (
    <article className="rounded-md border bg-card/70 p-3" aria-label={step.title}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 gap-2">
          <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><StepIcon kind={step.kind} /></span>
          <div className="min-w-0"><h3 className="text-sm font-semibold">{step.title}</h3><StepSummary step={step} /></div>
        </div>
        {focusEnvelope === null ? null : <Button variant="outline" size="sm" onClick={() => onFocusGraph(focusEnvelope)}>Focus in graph</Button>}
      </div>
      <StepContent step={step} onInspectCandidate={onInspectCandidate} />
      <TechnicalDetails step={step} onFocusGraph={onFocusGraph} />
    </article>
  )
}

function StepIcon({ kind }: { kind: SemanticStep["kind"] }): React.ReactElement {
  const className = "size-4"
  if (kind === "entity-mapping") return <DatabaseZap className={className} />
  if (kind === "graph-expansion") return <GitBranch className={className} />
  if (kind === "context-assembly") return <Braces className={className} />
  return <Sparkles className={className} />
}

function StepSummary({ step }: { step: SemanticStep }): React.ReactElement {
  if (step.kind === "entity-mapping") {
    const summary = step.summary
    return <p className="mt-1 text-xs text-muted-foreground">{summary.retrievedCount} retrieved · {summary.selectedCount} selected · {summary.excludedCount} excluded{summary.pendingCount === 0 ? "" : ` · ${summary.pendingCount} pending`}</p>
  }
  if (step.kind === "graph-expansion") {
    const total = Object.values(step.summary.selectedCounts).reduce((sum, count) => sum + count, 0)
    return <p className="mt-1 text-xs text-muted-foreground">{total} context records selected through graph expansion</p>
  }
  if (step.kind === "context-assembly") {
    const { tokensUsed, totalTokenBudget } = step.summary
    return <p className="mt-1 text-xs text-muted-foreground">{step.summary.sections.length} sections{tokensUsed === undefined ? "" : ` · ${tokensUsed.toLocaleString()}${totalTokenBudget === undefined ? "" : ` / ${totalTokenBudget.toLocaleString()}`} tokens`}</p>
  }
  return <p className="mt-1 text-xs text-muted-foreground">{step.summary.calls} {step.summary.calls === 1 ? "call" : "calls"} · {step.summary.inputTokens.toLocaleString()} input · {step.summary.outputTokens.toLocaleString()} output</p>
}

function StepContent({ step, onInspectCandidate }: { step: SemanticStep; onInspectCandidate: (candidate: ExplainabilityRecordView) => void }): React.ReactNode {
  if (step.kind === "entity-mapping") {
    const metadata = [
      step.summary.model,
      step.summary.elapsedMs === undefined ? undefined : `${step.summary.elapsedMs} ms`,
      step.summary.promptTokens === undefined ? undefined : `${step.summary.promptTokens} embedding tokens`,
      step.summary.dimensions === undefined ? undefined : `${step.summary.dimensions} dimensions`,
    ].filter((value): value is string => value !== undefined)
    return <div className="mt-3 space-y-3">{metadata.length === 0 ? null : <p className="text-[11px] text-muted-foreground">{metadata.join(" · ")}</p>}<DecisionRecordList records={step.summary.candidates} onInspectCandidate={onInspectCandidate} /></div>
  }
  if (step.kind === "graph-expansion") {
    const counts = Object.entries(step.summary.selectedCounts).filter(([, count]) => count > 0)
    return <div className="mt-3 space-y-3">{counts.length === 0 ? <p className="text-xs text-muted-foreground">No expansion records were selected.</p> : <dl className="grid grid-cols-2 gap-2 text-xs">{counts.map(([type, count]) => <div key={type} className="flex justify-between rounded border bg-muted/20 px-2 py-1.5"><dt>{RECORD_LABELS[type] ?? type.replaceAll("_", " ")}</dt><dd className="font-mono">{count}</dd></div>)}</dl>}<DecisionRecordList records={step.summary.records} onInspectCandidate={onInspectCandidate} /></div>
  }
  if (step.kind === "context-assembly") {
    return <div className="mt-3 space-y-2">{step.summary.sections.map((section) => <div key={`${section.section}:${section.name ?? ""}`} className="rounded border bg-muted/20 px-2 py-2 text-xs"><div className="flex items-center justify-between gap-3"><span className="font-medium capitalize">{section.name ?? section.section.replaceAll("_", " ")}</span><span>{section.selected_count} included</span></div><p className="mt-1 text-[11px] text-muted-foreground">{section.candidate_count} candidates · {section.tokens_used.toLocaleString()} / {section.token_budget.toLocaleString()} tokens{section.truncated ? " · truncated" : ""}</p></div>)}</div>
  }
  const summary = step.summary
  return <dl className="mt-3 grid grid-cols-2 gap-2 text-xs"><Metric label="Calls" value={summary.calls} /><Metric label="Input tokens" value={summary.inputTokens.toLocaleString()} /><Metric label="Output tokens" value={summary.outputTokens.toLocaleString()} /><Metric label="Latency" value={`${summary.elapsedMs.toLocaleString()} ms`} />{summary.model === undefined ? null : <Metric label="Model" value={summary.model} />}</dl>
}

function Metric({ label, value }: { label: string; value: string | number }): React.ReactElement {
  return <div className="rounded border bg-muted/20 px-2 py-1.5"><dt className="text-muted-foreground">{label}</dt><dd className="mt-0.5 font-medium">{value}</dd></div>
}

function DecisionRecordList({ records, onInspectCandidate }: { records: ExplainabilityRecordView[]; onInspectCandidate: (candidate: ExplainabilityRecordView) => void }): React.ReactElement | null {
  const [expanded, setExpanded] = useState(false)
  if (records.length === 0) return null
  const visible = expanded ? records : records.slice(0, 20)
  return (
    <div className="space-y-2">
      <div className="divide-y rounded-md border bg-background/50">
        {visible.map((record) => <DecisionRecordRow key={`${record.recordType}:${record.stableId}`} record={record} onInspectCandidate={onInspectCandidate} />)}
      </div>
      {!expanded && visible.length < records.length ? <Button variant="ghost" size="sm" onClick={() => setExpanded(true)}>Show all {records.length} records</Button> : null}
    </div>
  )
}

function DecisionRecordRow({ record, onInspectCandidate }: { record: ExplainabilityRecordView; onInspectCandidate: (candidate: ExplainabilityRecordView) => void }): React.ReactElement {
  const inspectable = (record.recordType === "entity" || record.recordType === "relationship") && record.stableId.length > 0
  const label = record.title ?? record.shortId ?? record.stableId
  return (
    <div className="flex items-center gap-2 px-2 py-2 text-xs">
      <DecisionIcon record={record} />
      <div className="min-w-0 flex-1">
        {inspectable ? <button type="button" className="max-w-full truncate text-left font-medium hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-label={`Inspect ${record.recordType} ${label}`} onClick={() => onInspectCandidate(record)}>{label}</button> : <span className="block truncate font-medium">{label}</span>}
        <span className="block truncate font-mono text-[10px] text-muted-foreground">{record.shortId === undefined ? record.stableId : `${record.shortId} · ${record.stableId}`}</span>
      </div>
      {record.score === undefined ? null : <span className="font-mono text-[11px] text-muted-foreground">{record.score.toFixed(4)}</span>}
      <Badge variant="outline">{decisionLabel(record)}</Badge>
    </div>
  )
}

function DecisionIcon({ record }: { record: ExplainabilityRecordView }): React.ReactElement {
  if (record.selectionStatus === "pending") return <Circle className="size-3.5 shrink-0 text-muted-foreground" aria-label="Retrieved; selection pending" />
  if (record.selectionStatus === "excluded") return <X className="size-3.5 shrink-0 text-muted-foreground" aria-label="Excluded" />
  if (record.finalContext === "included") return <Check className="size-3.5 shrink-0 text-success" aria-label="Included in final context" />
  return <Circle className="size-3.5 shrink-0 text-muted-foreground" aria-label={record.finalContext === "excluded" ? "Not in final context" : "Final context unknown"} />
}

function decisionLabel(record: ExplainabilityRecordView): string {
  if (record.selectionStatus === "pending") return "Retrieved"
  if (record.selectionStatus === "excluded") return "Excluded"
  if (record.finalContext === "included") return "Included"
  if (record.finalContext === "excluded") return "Not in final context"
  return "Selected · context unknown"
}

function TechnicalDetails({ step, onFocusGraph }: { step: SemanticStep; onFocusGraph: (envelope: ExplainabilityEnvelope) => void }): React.ReactElement {
  return (
    <Collapsible>
      <CollapsibleTrigger asChild><Button variant="ghost" size="sm" className="mt-3 px-1 text-muted-foreground">Technical details · {step.rawEvents.length} raw events</Button></CollapsibleTrigger>
      <CollapsibleContent className="mt-2 space-y-2 border-t pt-3">
        {step.rawEvents.map((envelope) => <TimelineEvent key={envelope.sequence} envelope={envelope} onFocusGraph={onFocusGraph} />)}
      </CollapsibleContent>
    </Collapsible>
  )
}
