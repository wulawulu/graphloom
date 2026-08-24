import { useState } from "react"
import { Braces, Check, Circle, Search, Sparkles } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityCandidate, ExplainabilityEnvelope } from "@/api/types"
import { CapturedContentViewer } from "@/components/explainability/captured-content-viewer"
import { TechnicalDetails } from "@/components/explainability/technical-details"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { BasicSemanticStep } from "@/lib/semantic-basic"
import { semanticStepTitle } from "@/i18n/presentation"
import type { StudioTranslationKey } from "@/i18n/types"

interface BasicSemanticStepCardProps {
  step: BasicSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

const INITIAL_CANDIDATES = 20

export function BasicSemanticStepCard({ step, onFocusGraph }: BasicSemanticStepCardProps): React.ReactElement {
  const { t } = useTranslation()
  const title = semanticStepTitle(t, step.kind)
  return (
    <article className="rounded-md border bg-card/70 p-3" aria-label={title}>
      <div className="flex min-w-0 gap-2">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><StepIcon kind={step.kind} /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{title}</h3><StepSummary step={step} /></div>
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
  const { t } = useTranslation()
  if (step.kind === "text-retrieval") {
    if (step.summary.status === "skipped") return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.retrievalSkippedEmptyQuery")}</p>
    if (step.summary.status === "embedding") return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.messages.embeddingQuery")}</p>
    if (step.summary.status === "embedding_ready") return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.embeddingReadyWaitingForAnnResults")}</p>
    if (step.summary.status === "waiting") return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.waitingForRetrieval")}</p>
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countTextUnitRetrieved", { count: step.summary.candidates.length })}</p>
  }
  if (step.kind === "basic-context-assembly") {
    const summary = step.summary
    if (summary.status === "waiting") return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.waitingForContextAssembly")}</p>
    if (summary.status === "assembling") return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.assemblingSourcesContext")}</p>
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.candidateInclusionSummary", { candidates: summary.candidateCount ?? summary.candidates.length, included: summary.selectedCount ?? 0 })}{summary.tokenBudgetExcludedCount === undefined ? "" : ` · ${t("explainability.counts.countExcludedByTokenBudget", { count: summary.tokenBudgetExcludedCount })}`}</p>
  }
  const status = t(step.summary.status === "generated" ? "explainability.labels.answerGenerated" : step.summary.status === "generating" ? "explainability.labels.generating" : "explainability.labels.waiting")
  return <p className="mt-1 text-xs text-muted-foreground">{status}{step.summary.model === undefined ? "" : ` · ${step.summary.model}`}</p>
}

function StepContent({ step }: { step: BasicSemanticStep }): React.ReactNode {
  const { t } = useTranslation()
  if (step.kind === "text-retrieval") {
    if (step.summary.status === "skipped") return <p className="mt-3 rounded border bg-muted/20 p-3 text-xs">{t("explainability.messages.emptyQuerySkipped")}</p>
    const metadata = [
      step.summary.model,
      step.summary.promptTokens === undefined ? undefined : t("explainability.counts.countEmbeddingToken", { count: step.summary.promptTokens }),
      step.summary.dimensions === undefined ? undefined : t("explainability.counts.countDimension", { count: step.summary.dimensions }),
      step.summary.elapsedMs === undefined ? undefined : `${step.summary.elapsedMs.toLocaleString()} ms`,
    ].filter((value): value is string => value !== undefined)
    return <div className="mt-3 min-w-0 space-y-3">{metadata.length === 0 ? null : <p className="text-[11px] text-muted-foreground">{metadata.join(" · ")}</p>}<CandidateList candidates={step.summary.candidates} mode="retrieved" /></div>
  }
  if (step.kind === "basic-context-assembly") {
    const summary = step.summary
    return (
      <div className="mt-3 min-w-0 space-y-3">
        {summary.status === "waiting" ? null : <p className="text-[11px] text-muted-foreground">{t("explainability.messages.basicSourceOrderNotice")}</p>}
        {summary.status === "completed" ? <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.labels.candidates" value={summary.candidateCount ?? summary.candidates.length} /><Metric label="graph.labels.included" value={summary.selectedCount ?? 0} />{summary.tokenBudgetExcludedCount === undefined ? null : <Metric label="explainability.labels.tokenBudgetExcluded" value={summary.tokenBudgetExcludedCount} />}{summary.budgetedTokensUsed === undefined ? null : <Metric label="explainability.labels.budgetedTokens" value={`${summary.budgetedTokensUsed.toLocaleString()}${summary.tokenBudget === undefined ? "" : ` / ${summary.tokenBudget.toLocaleString()}`}`} />}</dl> : null}
        <CandidateList candidates={summary.candidates} mode="decision" />
        {summary.status === "completed" ? <CapturedContentViewer buttonLabel="explainability.actions.viewBasicContext" title={t("explainability.labels.basicContext")} content={summary.exactContext} unavailableMessage="explainability.messages.basicContextNotCaptured" testId="exact-basic-context" exactTabLabel="explainability.labels.exactInput" copyLabel="explainability.actions.copyExactBasicContext" /> : null}
      </div>
    )
  }
  const summary = step.summary
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="answer.labels.calls" value={summary.calls} /><Metric label="explainability.labels.status" value={t(summary.status === "generated" ? "explainability.labels.answerGenerated" : summary.status === "generating" ? "explainability.labels.generating" : "explainability.labels.waiting")} />{summary.model === undefined ? null : <Metric label="explainability.labels.model" value={summary.model} />}{summary.inputTokens === undefined ? null : <Metric label="explainability.labels.inputTokens" value={summary.inputTokens.toLocaleString()} />}{summary.outputTokens === undefined ? null : <Metric label="explainability.labels.outputTokens" value={summary.outputTokens.toLocaleString()} />}{summary.elapsedMs === undefined ? null : <Metric label="explainability.labels.latency" value={`${summary.elapsedMs.toLocaleString()} ms`} />}</dl>
      {summary.status === "waiting" ? null : <div className="flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewBasicPrompt" title={t("explainability.labels.basicPrompt")} content={summary.exactPrompt} unavailableMessage="explainability.messages.basicPromptNotCaptured" testId="exact-basic-prompt" />{summary.status === "generated" ? <CapturedContentViewer buttonLabel="explainability.actions.viewRawBasicResponse" title={t("explainability.labels.rawBasicResponse")} content={summary.rawResponse} unavailableMessage="explainability.messages.basicRawResponseNotCaptured" testId="raw-basic-response" preview={false} /> : <span className="self-center text-xs text-muted-foreground">{t("explainability.labels.waitingForProviderResponse")}</span>}</div>}
    </div>
  )
}

function CandidateList({ candidates, mode }: { candidates: ExplainabilityCandidate[]; mode: "retrieved" | "decision" }): React.ReactElement | null {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  if (candidates.length === 0) return null
  const visible = showAll ? candidates : candidates.slice(0, INITIAL_CANDIDATES)
  return (
    <div className="min-w-0 space-y-2">
      <div className="divide-y overflow-hidden rounded border bg-background/50">{visible.map((candidate, index) => <CandidateRow key={`${candidate.id}:${candidate.rank ?? index}`} candidate={candidate} mode={mode} />)}</div>
      {candidates.length > INITIAL_CANDIDATES ? <Button variant="ghost" size="sm" aria-expanded={showAll} aria-label={showAll ? t("explainability.actions.showFewerBasicTextUnits") : t("explainability.counts.showAllCountBasicTextUnits", { count: candidates.length })} onClick={() => setShowAll((value) => !value)}>{showAll ? t("explainability.actions.showFewerTextUnits") : t("explainability.counts.showAllCountTextUnits", { count: candidates.length })}</Button> : null}
    </div>
  )
}

function CandidateRow({ candidate, mode }: { candidate: ExplainabilityCandidate; mode: "retrieved" | "decision" }): React.ReactElement {
  const { t } = useTranslation()
  const label = candidate.short_id === undefined ? candidate.id : t("explainability.labels.textUnitId", { id: candidate.short_id })
  const decision = t(candidate.selected ? "graph.labels.included" : candidate.reason === "token_budget" ? "explainability.labels.notIncludedAfterTokenBudgetStop" : "graph.labels.notIncluded")
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-x-2 gap-y-1 overflow-hidden px-2 py-2 text-xs">
      <span className="row-span-2 pt-0.5">{mode === "retrieved" || !candidate.selected ? <Circle className="size-3.5 text-muted-foreground" aria-label={mode === "retrieved" ? t("graph.labels.retrieved") : decision} /> : <Check className="size-3.5 text-success" aria-label={t("graph.labels.included")} />}</span>
      <div className="min-w-0"><p className="truncate font-medium" title={label}>{label}</p><p className="truncate font-mono text-[10px] text-muted-foreground" title={candidate.id}>{candidate.id}</p></div>
      <Badge variant="outline" className="max-w-48 shrink-0 truncate" title={mode === "retrieved" ? t("graph.labels.retrieved") : decision}>{mode === "retrieved" ? t("graph.labels.retrieved") : decision}</Badge>
      <div className="col-span-2 col-start-2 flex min-w-0 gap-3 text-[10px] text-muted-foreground">{candidate.rank === undefined ? null : <span>{t("explainability.labels.annRankValue", { value: candidate.rank })}</span>}{candidate.score === undefined ? null : <span>{t("explainability.labels.scoreValue", { value: candidate.score.toFixed(4) })}</span>}</div>
    </div>
  )
}

function Metric({ label, value }: { label: StudioTranslationKey; value: string | number }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="min-w-0 rounded border bg-muted/20 px-2 py-1.5"><dt className="text-muted-foreground">{t(label)}</dt><dd className="mt-0.5 break-words font-medium">{value}</dd></div>
}
