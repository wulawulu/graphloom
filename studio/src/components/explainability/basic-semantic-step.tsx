import { useEffect, useMemo, useState } from "react"
import { Braces, Check, Circle, Search, Sparkles } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityCandidate, ExplainabilityEnvelope } from "@/api/types"
import { useTextUnitEvidence } from "@/contexts/text-unit-evidence"
import { CapturedContentViewer } from "@/components/explainability/captured-content-viewer"
import { TechnicalDetails } from "@/components/explainability/technical-details"
import { TextUnitDetailSheet, type TextUnitDetailReference } from "@/components/graph/text-unit-detail-sheet"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import type { BasicSemanticStep } from "@/lib/semantic-basic"
import { semanticStepTitle } from "@/i18n/presentation"
import type { StudioTranslationKey } from "@/i18n/types"

interface BasicSemanticStepCardProps {
  step: BasicSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  showDeveloperDetails: boolean
}

const INITIAL_CANDIDATES = 20
const INITIAL_FINAL_SOURCES = 5

export function BasicSemanticStepCard({ step, onFocusGraph, showDeveloperDetails }: BasicSemanticStepCardProps): React.ReactElement {
  const { t } = useTranslation()
  const title = semanticStepTitle(t, step.kind)
  return (
    <article className="rounded-md border bg-card/70 p-3" aria-label={title}>
      <div className="flex min-w-0 gap-2">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><StepIcon kind={step.kind} /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{title}</h3><StepSummary step={step} /></div>
      </div>
      <StepContent step={step} showDeveloperDetails={showDeveloperDetails} />
      <TechnicalDetails visible={showDeveloperDetails} rawEvents={step.rawEvents} onFocusGraph={onFocusGraph} />
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

function StepContent({ step, showDeveloperDetails }: { step: BasicSemanticStep; showDeveloperDetails: boolean }): React.ReactNode {
  const { t } = useTranslation()
  if (step.kind === "text-retrieval") {
    if (step.summary.status === "skipped") return <p className="mt-3 rounded border bg-muted/20 p-3 text-xs">{t("explainability.messages.emptyQuerySkipped")}</p>
    const metadata = [
      step.summary.model,
      step.summary.promptTokens === undefined ? undefined : t("explainability.counts.countEmbeddingToken", { count: step.summary.promptTokens }),
      step.summary.dimensions === undefined ? undefined : t("explainability.counts.countDimension", { count: step.summary.dimensions }),
      step.summary.elapsedMs === undefined ? undefined : `${step.summary.elapsedMs.toLocaleString()} ms`,
    ].filter((value): value is string => value !== undefined)
    return <div className="mt-3 min-w-0 space-y-3">{metadata.length === 0 ? null : <p className="text-[11px] text-muted-foreground">{metadata.join(" · ")}</p>}<RetrievalCandidateList candidates={step.summary.candidates} /></div>
  }
  if (step.kind === "basic-context-assembly") {
    const summary = step.summary
    return (
      <div className="mt-3 min-w-0 space-y-3">
        {summary.status === "waiting" ? null : <p className="text-[11px] text-muted-foreground">{t("explainability.messages.basicSourceOrderNotice")}</p>}
        {summary.status === "completed" ? <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.basic.candidateSources" value={summary.candidateCount ?? summary.candidates.length} /><Metric label="explainability.basic.includedSources" value={summary.selectedCount ?? 0} />{summary.tokenBudgetExcludedCount === undefined ? null : <Metric label="explainability.basic.tokenBudgetCutoff" value={summary.tokenBudgetExcludedCount} />}{summary.budgetedTokensUsed === undefined ? null : <Metric label="explainability.basic.contextTokens" value={`${summary.budgetedTokensUsed.toLocaleString()}${summary.tokenBudget === undefined ? "" : ` / ${summary.tokenBudget.toLocaleString()}`}`} />}</dl> : null}
        {summary.selectedCount === undefined ? null : <ContextSources summary={summary} />}
        {summary.status === "completed" && showDeveloperDetails ? <CapturedContentViewer buttonLabel="explainability.actions.viewExactBasicContext" title={t("explainability.labels.basicContext")} content={summary.exactContext} testId="exact-basic-context" exactTabLabel="explainability.labels.exactInput" copyLabel="explainability.actions.copyExactBasicContext" /> : null}
      </div>
    )
  }
  const summary = step.summary
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.labels.status" value={t(summary.status === "generated" ? "explainability.labels.answerGenerated" : summary.status === "generating" ? "explainability.labels.generating" : "explainability.labels.waiting")} />{summary.model === undefined ? null : <Metric label="explainability.labels.model" value={summary.model} />}{summary.inputTokens === undefined ? null : <Metric label="explainability.labels.inputTokens" value={summary.inputTokens.toLocaleString()} />}{summary.outputTokens === undefined ? null : <Metric label="explainability.labels.outputTokens" value={summary.outputTokens.toLocaleString()} />}{summary.elapsedMs === undefined ? null : <Metric label="explainability.labels.latency" value={`${summary.elapsedMs.toLocaleString()} ms`} />}</dl>
      {summary.status === "waiting" ? null : <div className="flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewModelPrompt" title={t("explainability.labels.modelPrompt")} content={summary.exactPrompt} testId="exact-basic-prompt" />{summary.status === "generated" ? <CapturedContentViewer buttonLabel="explainability.actions.viewRawBasicResponse" title={t("explainability.labels.rawBasicResponse")} content={summary.rawResponse} testId="raw-basic-response" preview={false} /> : <span className="self-center text-xs text-muted-foreground">{t("explainability.labels.waitingForProviderResponse")}</span>}</div>}
    </div>
  )
}

function RetrievalCandidateList({ candidates }: { candidates: ExplainabilityCandidate[] }): React.ReactElement | null {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const [selected, setSelected] = useState<TextUnitDetailReference | null>(null)
  const { refs, resolve, status } = useTextUnitEvidence()
  const visible = useMemo(() => showAll ? candidates : candidates.slice(0, INITIAL_CANDIDATES), [candidates, showAll])
  const visibleIds = useMemo(() => visible.map((candidate) => candidate.id), [visible])
  useEffect(() => resolve(visibleIds), [resolve, visibleIds])
  if (candidates.length === 0) return null
  return (
    <div className="min-w-0 space-y-2">
      <div className="divide-y overflow-hidden rounded border bg-background/50">{visible.map((candidate, index) => <CandidateRow key={`${candidate.id}:${candidate.rank ?? index}`} candidate={candidate} preview={refs.get(candidate.id)?.preview} previewStatus={status(candidate.id)} onOpen={() => setSelected(detailReference(candidate, refs.get(candidate.id)))} />)}</div>
      {candidates.length > INITIAL_CANDIDATES ? <Button variant="ghost" size="sm" aria-expanded={showAll} aria-label={showAll ? t("explainability.actions.showFewerBasicTextUnits") : t("explainability.counts.showAllCountBasicTextUnits", { count: candidates.length })} onClick={() => setShowAll((value) => !value)}>{showAll ? t("explainability.actions.showFewerTextUnits") : t("explainability.counts.showAllCountTextUnits", { count: candidates.length })}</Button> : null}
      <TextUnitDetailSheet reference={selected} onClose={() => setSelected(null)} />
    </div>
  )
}

function CandidateRow({ candidate, preview, previewStatus, onOpen }: { candidate: ExplainabilityCandidate; preview: string | undefined; previewStatus: "idle" | "loading" | "resolved" | "unavailable"; onOpen: () => void }): React.ReactElement {
  const { t } = useTranslation()
  const label = candidateLabel(candidate, t)
  const fallbackId = candidate.short_id === undefined ? compactStableId(candidate.id) : null
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-x-2 gap-y-1 overflow-hidden px-2 py-2 text-xs">
      <span className="row-span-4 pt-0.5"><Circle className="size-3.5 text-muted-foreground" aria-label={t("graph.labels.retrieved")} /></span>
      <div className="min-w-0"><p className="break-words font-medium">{label}</p>{fallbackId === null ? null : <p className="break-all font-mono text-[10px] text-muted-foreground">{t("explainability.labels.shortStableId", { id: fallbackId })}</p>}</div>
      <Badge variant="outline" className="max-w-48 shrink-0 truncate" title={t("graph.labels.retrieved")}>{t("graph.labels.retrieved")}</Badge>
      <div className="col-span-2 col-start-2 flex min-w-0 gap-3 text-[10px] text-muted-foreground">{candidate.rank === undefined ? null : <span>{t("explainability.labels.annRankValue", { value: candidate.rank })}</span>}{candidate.score === undefined ? null : <span>{t("explainability.labels.scoreValue", { value: candidate.score.toFixed(4) })}</span>}</div>
      <p className="col-span-2 col-start-2 mt-1 line-clamp-3 min-w-0 whitespace-normal break-words text-xs leading-5 text-muted-foreground">{preview ?? t(previewStatus === "unavailable" ? "explainability.sources.previewUnavailable" : "explainability.sources.loadingPreview")}</p>
      <Button variant="link" size="sm" className="col-start-3 h-auto justify-self-end px-0 py-0 text-[11px]" onClick={onOpen}>{t("explainability.actions.viewSource")}</Button>
    </div>
  )
}

function ContextSources({ summary }: { summary: Extract<BasicSemanticStep, { kind: "basic-context-assembly" }>['summary'] }): React.ReactElement {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const [selected, setSelected] = useState<TextUnitDetailReference | null>(null)
  const { refs, resolve, status } = useTextUnitEvidence()
  const candidatesById = useMemo(() => new Map(summary.candidates.map((candidate) => [candidate.id, candidate])), [summary.candidates])
  const finalSources = useMemo(() => summary.selectedRecordIds.flatMap((id) => {
    const candidate = candidatesById.get(id)
    return candidate === undefined ? [] : [candidate]
  }), [candidatesById, summary.selectedRecordIds])
  const selectedIds = useMemo(() => new Set(summary.selectedRecordIds), [summary.selectedRecordIds])
  const excluded = useMemo(() => summary.candidates.filter((candidate) => !selectedIds.has(candidate.id)), [selectedIds, summary.candidates])
  const visible = showAll ? finalSources : finalSources.slice(0, INITIAL_FINAL_SOURCES)
  const visibleIds = useMemo(() => visible.map((candidate) => candidate.id), [visible])
  useEffect(() => resolve(visibleIds), [resolve, visibleIds])
  return (
    <div className="min-w-0 space-y-3">
      <h4 className="text-xs font-semibold">{t("explainability.basic.finalSources")}</h4>
      {finalSources.length === 0 ? <p className="text-xs text-muted-foreground">{t("explainability.basic.noFinalSources")}</p> : (
        <div className="divide-y overflow-hidden rounded border bg-background/50">
          {visible.map((candidate) => {
            const reference = refs.get(candidate.id)
            return <FinalSourceRow key={candidate.id} candidate={candidate} preview={reference?.preview} previewStatus={status(candidate.id)} onOpen={() => setSelected(detailReference(candidate, reference))} />
          })}
        </div>
      )}
      {finalSources.length > INITIAL_FINAL_SOURCES ? <Button variant="ghost" size="sm" aria-expanded={showAll} onClick={() => setShowAll((value) => !value)}>{showAll ? t("explainability.actions.showFewerTextUnits") : t("explainability.basic.viewAllFinalSources", { count: finalSources.length })}</Button> : null}
      {excluded.length === 0 ? null : <ExcludedSources candidates={excluded} />}
      <TextUnitDetailSheet reference={selected} onClose={() => setSelected(null)} />
    </div>
  )
}

function FinalSourceRow({ candidate, preview, previewStatus, onOpen }: { candidate: ExplainabilityCandidate; preview: string | undefined; previewStatus: "idle" | "loading" | "resolved" | "unavailable"; onOpen: () => void }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-x-2 gap-y-1 px-2 py-2 text-xs">
      <Check className="mt-0.5 size-3.5 text-success" aria-label={t("graph.labels.included")} />
      <p className="break-words font-medium">{candidateLabel(candidate, t)}</p>
      <Button variant="link" size="sm" className="h-auto px-0 py-0 text-[11px]" onClick={onOpen}>{t("explainability.actions.viewSource")}</Button>
      <p className="col-span-2 col-start-2 line-clamp-2 min-w-0 whitespace-normal break-words text-xs leading-5 text-muted-foreground">{preview ?? t(previewStatus === "unavailable" ? "explainability.sources.previewUnavailable" : "explainability.sources.loadingPreview")}</p>
    </div>
  )
}

function ExcludedSources({ candidates }: { candidates: ExplainabilityCandidate[] }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <Collapsible>
      <CollapsibleTrigger asChild><Button variant="ghost" size="sm">{t("explainability.basic.viewExcludedSources", { count: candidates.length })}</Button></CollapsibleTrigger>
      <CollapsibleContent className="mt-1 rounded border bg-muted/10 px-3 py-2">
        <ul className="space-y-2">{candidates.map((candidate) => <li key={candidate.id} className="flex min-w-0 items-center justify-between gap-3 text-xs"><span className="min-w-0 break-words font-medium">{candidateLabel(candidate, t)}</span><span className="shrink-0 text-[10px] text-muted-foreground">{t(candidate.reason === "token_budget" ? "explainability.basic.tokenBudgetCutoff" : "graph.labels.notIncluded")}</span></li>)}</ul>
      </CollapsibleContent>
    </Collapsible>
  )
}

function detailReference(candidate: ExplainabilityCandidate, reference: { short_id: string; n_tokens: number | null } | undefined): TextUnitDetailReference {
  return { id: candidate.id, shortId: reference?.short_id ?? candidate.short_id, nTokens: reference?.n_tokens }
}

function candidateLabel(candidate: ExplainabilityCandidate, t: ReturnType<typeof useTranslation>["t"]): string {
  return candidate.short_id === undefined ? t("explainability.labels.textUnit") : t("explainability.labels.textUnitId", { id: candidate.short_id })
}

function compactStableId(id: string): string {
  if (id.length <= 16) return id
  return `${id.slice(0, 8)}…${id.slice(-8)}`
}

function Metric({ label, value }: { label: StudioTranslationKey; value: string | number }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="min-w-0 rounded border bg-muted/20 px-2 py-1.5"><dt className="text-muted-foreground">{t(label)}</dt><dd className="mt-0.5 break-words font-medium">{value}</dd></div>
}
