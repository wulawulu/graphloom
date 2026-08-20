import { useState } from "react"
import { Braces, Check, Circle, Search, Sparkles } from "lucide-react"

import type { ExplainabilityCandidate, ExplainabilityEnvelope } from "@/api/types"
import { CapturedContentViewer } from "@/components/explainability/captured-content-viewer"
import { TechnicalDetails } from "@/components/explainability/technical-details"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { BasicSemanticStep } from "@/lib/semantic-basic"

interface BasicSemanticStepCardProps {
  step: BasicSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

const INITIAL_CANDIDATES = 20

export function BasicSemanticStepCard({ step, onFocusGraph }: BasicSemanticStepCardProps): React.ReactElement {
  return (
    <article className="rounded-md border bg-card/70 p-3" aria-label={step.title}>
      <div className="flex min-w-0 gap-2">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><StepIcon kind={step.kind} /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{step.title}</h3><StepSummary step={step} /></div>
      </div>
      <StepContent step={step} />
      <TechnicalDetails rawEvents={step.rawEvents} onFocusGraph={onFocusGraph} />
    </article>
  )
}

function StepIcon({ kind }: { kind: BasicSemanticStep["kind"] }): React.ReactElement {
  if (kind === "text-retrieval") return <Search className="size-4" />
  if (kind === "basic-context-assembly") return <Braces className="size-4" />
  return <Sparkles className="size-4" />
}

function StepSummary({ step }: { step: BasicSemanticStep }): React.ReactElement {
  if (step.kind === "text-retrieval") {
    if (step.summary.status === "skipped") return <p className="mt-1 text-xs text-muted-foreground">Retrieval skipped — empty query</p>
    if (step.summary.status === "embedding") return <p className="mt-1 text-xs text-muted-foreground">Embedding query...</p>
    if (step.summary.status === "embedding_ready") return <p className="mt-1 text-xs text-muted-foreground">Embedding ready · waiting for ANN results</p>
    if (step.summary.status === "waiting") return <p className="mt-1 text-xs text-muted-foreground">Waiting for retrieval</p>
    return <p className="mt-1 text-xs text-muted-foreground">{step.summary.candidates.length} text {step.summary.candidates.length === 1 ? "unit" : "units"} retrieved</p>
  }
  if (step.kind === "basic-context-assembly") {
    const summary = step.summary
    if (summary.status === "waiting") return <p className="mt-1 text-xs text-muted-foreground">Waiting for context assembly</p>
    if (summary.status === "assembling") return <p className="mt-1 text-xs text-muted-foreground">Assembling Sources context</p>
    return <p className="mt-1 text-xs text-muted-foreground">{summary.candidateCount ?? summary.candidates.length} candidates · {summary.selectedCount ?? 0} included{summary.tokenBudgetExcludedCount === undefined ? "" : ` · ${summary.tokenBudgetExcludedCount} excluded by token budget`}</p>
  }
  const status = step.summary.status === "generated" ? "Answer generated" : step.summary.status === "generating" ? "Generating" : "Waiting"
  return <p className="mt-1 text-xs text-muted-foreground">{status}{step.summary.model === undefined ? "" : ` · ${step.summary.model}`}</p>
}

function StepContent({ step }: { step: BasicSemanticStep }): React.ReactNode {
  if (step.kind === "text-retrieval") {
    if (step.summary.status === "skipped") return <p className="mt-3 rounded border bg-muted/20 p-3 text-xs">Embedding and ANN search were not invoked because the query was empty.</p>
    const metadata = [
      step.summary.model,
      step.summary.promptTokens === undefined ? undefined : `${step.summary.promptTokens.toLocaleString()} embedding tokens`,
      step.summary.dimensions === undefined ? undefined : `${step.summary.dimensions.toLocaleString()} dimensions`,
      step.summary.elapsedMs === undefined ? undefined : `${step.summary.elapsedMs.toLocaleString()} ms`,
    ].filter((value): value is string => value !== undefined)
    return <div className="mt-3 min-w-0 space-y-3">{metadata.length === 0 ? null : <p className="text-[11px] text-muted-foreground">{metadata.join(" · ")}</p>}<CandidateList candidates={step.summary.candidates} mode="retrieved" /></div>
  }
  if (step.kind === "basic-context-assembly") {
    const summary = step.summary
    return (
      <div className="mt-3 min-w-0 space-y-3">
        {summary.status === "waiting" ? null : <p className="text-[11px] text-muted-foreground">Basic preserves source-table order after ANN matching; token fitting stops at the first over-budget row.</p>}
        {summary.status === "completed" ? <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="Candidates" value={summary.candidateCount ?? summary.candidates.length} /><Metric label="Included" value={summary.selectedCount ?? 0} />{summary.tokenBudgetExcludedCount === undefined ? null : <Metric label="Token-budget excluded" value={summary.tokenBudgetExcludedCount} />}{summary.budgetedTokensUsed === undefined ? null : <Metric label="Budgeted tokens" value={`${summary.budgetedTokensUsed.toLocaleString()}${summary.tokenBudget === undefined ? "" : ` / ${summary.tokenBudget.toLocaleString()}`}`} />}</dl> : null}
        <CandidateList candidates={summary.candidates} mode="decision" />
        {summary.status === "completed" ? <CapturedContentViewer buttonLabel="View Basic Context" title="Basic Context" content={summary.exactContext} unavailableMessage="Basic context content was not captured. Run with Content or Debug to inspect exact input." testId="exact-basic-context" exactTabLabel="Exact input" copyLabel="Copy exact Basic context" /> : null}
      </div>
    )
  }
  const summary = step.summary
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="Calls" value={summary.calls} /><Metric label="Status" value={summary.status === "generated" ? "Answer generated" : summary.status === "generating" ? "Generating" : "Waiting"} />{summary.model === undefined ? null : <Metric label="Model" value={summary.model} />}{summary.inputTokens === undefined ? null : <Metric label="Input tokens" value={summary.inputTokens.toLocaleString()} />}{summary.outputTokens === undefined ? null : <Metric label="Output tokens" value={summary.outputTokens.toLocaleString()} />}{summary.elapsedMs === undefined ? null : <Metric label="Latency" value={`${summary.elapsedMs.toLocaleString()} ms`} />}</dl>
      {summary.status === "waiting" ? null : <div className="flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="View Basic Prompt" title="Basic Prompt" content={summary.exactPrompt} unavailableMessage="Basic prompt content was not captured. Run with Content or Debug to inspect the exact rendered prompt." testId="exact-basic-prompt" />{summary.status === "generated" ? <CapturedContentViewer buttonLabel="View Raw Basic Response" title="Raw Basic Response" content={summary.rawResponse} unavailableMessage="Raw Basic response content was not captured. Run with Content or Debug to inspect the provider response." testId="raw-basic-response" preview={false} /> : <span className="self-center text-xs text-muted-foreground">Waiting for provider response</span>}</div>}
    </div>
  )
}

function CandidateList({ candidates, mode }: { candidates: ExplainabilityCandidate[]; mode: "retrieved" | "decision" }): React.ReactElement | null {
  const [showAll, setShowAll] = useState(false)
  if (candidates.length === 0) return null
  const visible = showAll ? candidates : candidates.slice(0, INITIAL_CANDIDATES)
  return (
    <div className="min-w-0 space-y-2">
      <div className="divide-y overflow-hidden rounded border bg-background/50">{visible.map((candidate, index) => <CandidateRow key={`${candidate.id}:${candidate.rank ?? index}`} candidate={candidate} mode={mode} />)}</div>
      {candidates.length > INITIAL_CANDIDATES ? <Button variant="ghost" size="sm" aria-expanded={showAll} aria-label={showAll ? "Show fewer Basic text units" : `Show all ${candidates.length} Basic text units`} onClick={() => setShowAll((value) => !value)}>{showAll ? "Show fewer text units" : `Show all ${candidates.length} text units`}</Button> : null}
    </div>
  )
}

function CandidateRow({ candidate, mode }: { candidate: ExplainabilityCandidate; mode: "retrieved" | "decision" }): React.ReactElement {
  const label = candidate.short_id === undefined ? candidate.id : `Text Unit ${candidate.short_id}`
  const decision = candidate.selected ? "Included" : candidate.reason === "token_budget" ? "Not included after token-budget stop" : "Not included"
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-x-2 gap-y-1 overflow-hidden px-2 py-2 text-xs">
      <span className="row-span-2 pt-0.5">{mode === "retrieved" || !candidate.selected ? <Circle className="size-3.5 text-muted-foreground" aria-label={mode === "retrieved" ? "Retrieved" : decision} /> : <Check className="size-3.5 text-success" aria-label="Included" />}</span>
      <div className="min-w-0"><p className="truncate font-medium" title={label}>{label}</p><p className="truncate font-mono text-[10px] text-muted-foreground" title={candidate.id}>{candidate.id}</p></div>
      <Badge variant="outline" className="max-w-48 shrink-0 truncate" title={mode === "retrieved" ? "Retrieved" : decision}>{mode === "retrieved" ? "Retrieved" : decision}</Badge>
      <div className="col-span-2 col-start-2 flex min-w-0 gap-3 text-[10px] text-muted-foreground">{candidate.rank === undefined ? null : <span>ANN rank {candidate.rank}</span>}{candidate.score === undefined ? null : <span>Score {candidate.score.toFixed(4)}</span>}</div>
    </div>
  )
}

function Metric({ label, value }: { label: string; value: string | number }): React.ReactElement {
  return <div className="min-w-0 rounded border bg-muted/20 px-2 py-1.5"><dt className="text-muted-foreground">{label}</dt><dd className="mt-0.5 break-words font-medium">{value}</dd></div>
}
