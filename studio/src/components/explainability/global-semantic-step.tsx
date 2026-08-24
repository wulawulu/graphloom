import { useState } from "react"
import type { TFunction } from "i18next"
import { Check, Circle, Files, Filter, GitBranch, Sparkles, Split, X } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityEnvelope } from "@/api/types"
import { CapturedContentViewer } from "@/components/explainability/captured-content-viewer"
import { TechnicalDetails } from "@/components/explainability/technical-details"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import type {
  DynamicCommunityDecisionView,
  DynamicCommunitySelectionSummary,
  DynamicRatingAttemptView,
  DynamicTraversalWaveView,
  GlobalMapBatchView,
  GlobalMapPointView,
  GlobalReduceDecisionView,
  GlobalSemanticStep,
} from "@/lib/semantic-global"
import { semanticStepTitle } from "@/i18n/presentation"
import type { StudioTranslationKey } from "@/i18n/types"

interface GlobalSemanticStepCardProps {
  step: GlobalSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  showDeveloperDetails: boolean
}

const INITIAL_BATCHES = 6
const INITIAL_POINTS = 20
const INITIAL_COMMUNITIES = 20
const INITIAL_WAVES = 5
const WAVE_ID_PAGE_SIZE = 20

export function GlobalSemanticStepCard({ step, onFocusGraph, showDeveloperDetails }: GlobalSemanticStepCardProps): React.ReactElement {
  const { t } = useTranslation()
  const title = semanticStepTitle(t, step.kind)
  return (
    <article className="min-w-0 rounded-md border bg-card/70 p-3" aria-label={title}>
      <div className="flex min-w-0 items-start gap-2">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><GlobalStepIcon kind={step.kind} /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{title}</h3><GlobalStepSummary step={step} /></div>
      </div>
      <GlobalStepContent step={step} />
      <TechnicalDetails visible={showDeveloperDetails} rawEvents={step.rawEvents} onFocusGraph={onFocusGraph} />
    </article>
  )
}

function GlobalStepIcon({ kind }: { kind: GlobalSemanticStep["kind"] }): React.ReactElement {
  const className = "size-4"
  if (kind === "community-selection") return <GitBranch className={className} />
  if (kind === "community-context") return <Files className={className} />
  if (kind === "map-analysis") return <Split className={className} />
  if (kind === "evidence-reduction") return <Filter className={className} />
  return <Sparkles className={className} />
}

function GlobalStepSummary({ step }: { step: GlobalSemanticStep }): React.ReactElement {
  const { t } = useTranslation()
  if (step.kind === "community-selection") {
    const summary = step.summary
    if (summary.completed) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countCommunityRated", { count: summary.visitedCount ?? 0 })} · {t("explainability.counts.countSelected", { count: summary.selectedCount ?? 0 })}{summary.threshold === undefined ? "" : ` · ${t("explainability.status.thresholdValue", { value: summary.threshold })}`}{summary.numRepeats === undefined ? "" : ` · ${t("explainability.counts.countRepeat", { count: summary.numRepeats })}`}</p>
    if (summary.activeWave !== undefined) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.ratingAttemptProgress", { wave: summary.activeWave + 1, completed: summary.attemptsCompleted, started: summary.attemptsStarted })}</p>
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.actions.selectingCommunities")}</p>
  }
  if (step.kind === "community-context") {
    const summary = step.summary
    if (!summary.built) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.waitingForCommunityContext")}</p>
    const batchProgress = summary.batchCount === undefined || summary.batches.length === summary.batchCount
      ? `${t("explainability.counts.countMapBatch", { count: summary.batchCount ?? summary.batches.length })} · ${t("explainability.counts.countToken", { count: summary.tokensUsed })}`
      : t("explainability.labels.readyTotalBatchContextsReady", { ready: summary.batches.length, total: summary.batchCount })
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countCommunityReport", { count: summary.reportCount ?? 0 })} · {batchProgress}</p>
  }
  if (step.kind === "map-analysis") {
    const summary = step.summary
    if (!summary.started) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.waitingForMapAnalysis")}</p>
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countBatch", { count: summary.batchCount ?? 0 })} · {t("explainability.counts.countAnalystCall", { count: summary.analystCalls })} · {t("explainability.counts.countPoint", { count: summary.pointCount })} · {t("explainability.counts.countPositive", { count: summary.positivePointCount })}</p>
  }
  if (step.kind === "evidence-reduction") {
    const summary = step.summary
    if (!summary.built) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.waitingForEvidenceReduction")}</p>
    if (summary.skippedReason === "no_positive_points") return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countCandidatePointNoPositiveEvidenceReduceLlmSkipped", { count: summary.candidatePointCount ?? 0 })}</p>
    const tokens = summary.tokensUsed === undefined ? "" : ` · ${summary.tokensUsed.toLocaleString()}${summary.tokenBudget === undefined ? "" : ` / ${summary.tokenBudget.toLocaleString()}`} ${t("explainability.status.tokens")}`
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.countCandidate", { count: summary.candidatePointCount ?? 0 })} · {t("explainability.counts.countPositive", { count: summary.positivePointCount ?? 0 })} · {t("explainability.counts.countIncluded", { count: summary.selectedPointCount ?? 0 })}{tokens}{summary.truncated ? ` · ${t("explainability.status.truncated")}` : ""}</p>
  }
  const summary = step.summary
  if (summary.noDataAnswerReturned) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.noDataAnswerReturnedReduceLlmNotInvoked")}</p>
  if (summary.noDataPathSelected) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.noDataPathSelectedReduceLlmSkipped")}</p>
  if (summary.generated) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.answerGenerated")} · {t("answer.counts.countInput", { count: summary.inputTokens ?? 0 })} · {t("answer.counts.countOutput", { count: summary.outputTokens ?? 0 })}</p>
  return <p className="mt-1 text-xs text-muted-foreground">{t(summary.calls === 0 ? "explainability.labels.waitingForReduceLlm" : "explainability.labels.reduceLlmRunning")}</p>
}

function GlobalStepContent({ step }: { step: GlobalSemanticStep }): React.ReactNode {
  if (step.kind === "community-selection") return <CommunitySelectionContent summary={step.summary} />
  if (step.kind === "community-context") return <CommunityContextContent batches={step.summary.batches} />
  if (step.kind === "map-analysis") return <MapAnalysisContent batches={step.summary.batches} />
  if (step.kind === "evidence-reduction") return <EvidenceReductionContent summary={step.summary} />
  return <AnswerGenerationContent summary={step.summary} />
}

function CommunitySelectionContent({ summary }: { summary: DynamicCommunitySelectionSummary }): React.ReactElement {
  const { t } = useTranslation()
  const [showAllCommunities, setShowAllCommunities] = useState(false)
  const [showAllWaves, setShowAllWaves] = useState(false)
  const visibleCommunities = showAllCommunities ? summary.decisions : summary.decisions.slice(0, INITIAL_COMMUNITIES)
  const visibleWaves = showAllWaves ? summary.waves : summary.waves.slice(0, INITIAL_WAVES)
  return (
    <div className="mt-3 min-w-0 space-y-4">
      <dl className="grid grid-cols-2 gap-2 text-xs">
        {summary.initialCommunityCount === undefined ? null : <Metric label="explainability.labels.initialCommunities" value={summary.initialCommunityCount} />}
        {summary.threshold === undefined ? null : <Metric label="explainability.labels.threshold" value={summary.threshold} />}
        {summary.numRepeats === undefined ? null : <Metric label="explainability.labels.repeats" value={summary.numRepeats} />}
        {summary.maxLevel === undefined ? null : <Metric label="explainability.labels.fallbackMaxLevel" value={summary.maxLevel} />}
        {summary.visitedCount === undefined ? null : <Metric label="explainability.labels.communitiesRated" value={summary.visitedCount} />}
        {summary.thresholdPassedCount === undefined ? null : <Metric label="explainability.labels.passedThreshold" value={summary.thresholdPassedCount} />}
        {summary.selectedCount === undefined ? null : <Metric label="explainability.actions.selected" value={summary.selectedCount} />}
        <Metric label="explainability.labels.ratingAttempts" value={t("explainability.labels.completedStartedResponses", { completed: summary.attemptsCompleted, started: summary.attemptsStarted })} />
      </dl>
      <section aria-label={t("explainability.labels.dynamicTraversalWaves")} className="min-w-0"><h4 className="text-xs font-semibold">{t("explainability.labels.traversalWaves")}</h4><div className="mt-2 space-y-2">{visibleWaves.map((wave) => <TraversalWaveRow key={`${wave.waveIndex}:${wave.source}`} wave={wave} />)}</div>{summary.waves.length > INITIAL_WAVES ? <Button variant="ghost" size="sm" onClick={() => setShowAllWaves((value) => !value)}>{showAllWaves ? t("explainability.actions.showFewerWaves") : t("explainability.counts.showAllCountWaves", { count: summary.waves.length })}</Button> : null}</section>
      <section aria-label={t("explainability.labels.dynamicCommunityDecisions")} className="min-w-0"><h4 className="text-xs font-semibold">{t("explainability.labels.communityDecisions")}</h4>{summary.completed ? <><div className="mt-2 space-y-2">{visibleCommunities.map((decision) => <CommunityDecisionRow key={decision.community_id} decision={decision} />)}</div>{summary.decisions.length > INITIAL_COMMUNITIES ? <Button variant="ghost" size="sm" onClick={() => setShowAllCommunities((value) => !value)}>{showAllCommunities ? t("explainability.actions.showFewerCommunities") : t("explainability.counts.showAllCountCommunities", { count: summary.decisions.length })}</Button> : null}</> : <p className="mt-2 text-xs text-muted-foreground">{t("explainability.messages.communityDecisionsPending")}</p>}</section>
    </div>
  )
}

function TraversalWaveRow({ wave }: { wave: DynamicTraversalWaveView }): React.ReactElement {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [visibleIdCount, setVisibleIdCount] = useState(WAVE_ID_PAGE_SIZE)
  const visibleIds = wave.communityIds.slice(0, visibleIdCount)
  const remainingIdCount = wave.communityIds.length - visibleIds.length
  return <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50"><CollapsibleTrigger asChild><button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={t(open ? "explainability.actions.hideWaveCommunityIds" : "explainability.actions.showWaveCommunityIds", { wave: wave.waveIndex + 1 })}><span className="font-medium">{t("explainability.labels.traversalWave", { wave: wave.waveIndex + 1, source: t(waveSourceLabel(wave.source)) })}</span><span className="shrink-0 text-muted-foreground">{t("graph.counts.countCommunity", { count: wave.communityIds.length })}</span></button></CollapsibleTrigger><CollapsibleContent className="border-t p-3"><ul className="max-h-32 overflow-auto rounded border bg-muted/20 p-2 font-mono text-[11px]">{visibleIds.map((id) => <li key={id} className="break-all">{id}</li>)}</ul><div className="mt-1 flex flex-wrap gap-1">{remainingIdCount > 0 ? <Button variant="ghost" size="sm" onClick={() => setVisibleIdCount((count) => Math.min(count + WAVE_ID_PAGE_SIZE, wave.communityIds.length))}>{t("explainability.counts.showNextCountCommunityIds", { count: Math.min(WAVE_ID_PAGE_SIZE, remainingIdCount) })}</Button> : null}{visibleIdCount > WAVE_ID_PAGE_SIZE ? <Button variant="ghost" size="sm" onClick={() => setVisibleIdCount(WAVE_ID_PAGE_SIZE)}>{t("explainability.actions.showFewerCommunityIds")}</Button> : null}</div></CollapsibleContent></Collapsible>
}

function CommunityDecisionRow({ decision }: { decision: DynamicCommunityDecisionView }): React.ReactElement {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const status = t(dynamicDecisionLabel(decision))
  return <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50"><CollapsibleTrigger asChild><button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={t(open ? "explainability.actions.collapseRatingDetailsForCommunityId" : "explainability.actions.expandRatingDetailsForCommunityId", { id: decision.community_id })}><span className="min-w-0"><span className="block truncate font-medium">{t("explainability.labels.communityId", { id: decision.community_id })}</span><span className="text-muted-foreground">{t("explainability.labels.communityRating", { level: decision.level, rating: decision.selected_rating })}</span></span><Badge variant="outline" className="shrink-0">{status}</Badge></button></CollapsibleTrigger><CollapsibleContent className="min-w-0 space-y-3 border-t p-3"><p className="font-mono text-[10px] text-muted-foreground break-all">report_id {decision.report_id}</p><h5 className="text-xs font-semibold">{t("explainability.labels.ratingAttempts")}</h5>{decision.attempts.length === 0 ? <p className="text-xs text-muted-foreground">{t("explainability.messages.ratingAttemptDetailsAreNotAvailable")}</p> : <div className="space-y-2">{decision.attempts.map((attempt) => <RatingAttemptRow key={attempt.identity} attempt={attempt} />)}</div>}</CollapsibleContent></Collapsible>
}

function RatingAttemptRow({ attempt }: { attempt: DynamicRatingAttemptView }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="min-w-0 space-y-2 rounded border bg-muted/10 p-2 text-xs"><div className="flex min-w-0 items-center justify-between gap-2"><span className="font-medium">{t("explainability.labels.repeatValue", { value: attempt.repeatIndex + 1 })}</span><Badge variant="outline">{t(attempt.status === "response_received" ? "explainability.labels.responseReceived" : "explainability.labels.rating")}</Badge></div><dl className="grid grid-cols-2 gap-2"><Metric label="explainability.labels.model" value={attempt.model ?? t("explainability.labels.notAvailableYet")} />{attempt.inputTokens === undefined ? null : <Metric label="explainability.labels.inputTokens" value={attempt.inputTokens.toLocaleString()} />}{attempt.outputTokens === undefined ? null : <Metric label="explainability.labels.outputTokens" value={attempt.outputTokens.toLocaleString()} />}{attempt.elapsedMs === undefined ? null : <Metric label="explainability.labels.latency" value={`${attempt.elapsedMs.toLocaleString()} ms`} />}</dl><div className="flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewRatingPrompt" title={t("explainability.labels.ratingPromptTitle", { community: attempt.communityId, repeat: attempt.repeatIndex + 1 })} content={attempt.exactPrompt} unavailableMessage="explainability.messages.ratingPromptNotCaptured" testId={`rating-prompt-${attempt.communityId}-${attempt.repeatIndex}`} /><CapturedContentViewer buttonLabel="explainability.actions.viewRawRatingResponse" title={t("explainability.labels.ratingRawResponseTitle", { community: attempt.communityId, repeat: attempt.repeatIndex + 1 })} content={attempt.rawResponse} unavailableMessage="explainability.messages.ratingRawResponseNotCaptured" testId={`rating-response-${attempt.communityId}-${attempt.repeatIndex}`} preview={false} /></div></div>
}

function CommunityContextContent({ batches }: { batches: GlobalMapBatchView[] }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="mt-3 min-w-0">
      {batches.length === 0 ? <p className="text-xs text-muted-foreground">{t("explainability.messages.waitingForCommunityContextBatches")}</p> : <LimitedBatchList batches={batches} renderBatch={(batch) => <CommunityBatch key={batch.batchIndex} batch={batch} />} />}
    </div>
  )
}

function CommunityBatch({ batch }: { batch: GlobalMapBatchView }): React.ReactElement {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50">
      <CollapsibleTrigger asChild>
        <button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={t(open ? "explainability.actions.collapseMapBatch" : "explainability.actions.expandMapBatch", { batch: batch.batchIndex + 1 })}>
          <span className="font-medium">{t("explainability.labels.batchValue", { value: batch.batchIndex + 1 })}</span>
          <span className="shrink-0 text-muted-foreground">{t("graph.counts.countReport", { count: batch.reportCount })} · {batch.tokensUsed.toLocaleString()} / {batch.tokenBudget.toLocaleString()} {t("explainability.status.tokens")}</span>
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent className="min-w-0 space-y-3 border-t p-3">
        <p className="font-mono text-[10px] text-muted-foreground">batch_index {batch.batchIndex}</p>
        <div><p className="text-[11px] font-semibold text-muted-foreground uppercase">{t("explainability.labels.stableReportIds")}</p><ul className="mt-1 max-h-32 overflow-auto rounded border bg-muted/20 p-2 font-mono text-[11px]">{batch.reportIds.map((id) => <li key={id} className="break-all">{id}</li>)}</ul></div>
        <CapturedContentViewer buttonLabel="explainability.actions.viewMapContext" title={t("explainability.labels.mapContextTitle", { batch: batch.batchIndex + 1 })} content={batch.exactContext} unavailableMessage="explainability.messages.mapContextNotCaptured" testId={`exact-map-context-${batch.batchIndex}`} />
      </CollapsibleContent>
    </Collapsible>
  )
}

function MapAnalysisContent({ batches }: { batches: GlobalMapBatchView[] }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="mt-3 min-w-0">
      {batches.length === 0 ? <p className="text-xs text-muted-foreground">{t("explainability.messages.waitingForMapAnalysis")}</p> : <LimitedBatchList batches={batches} renderBatch={(batch) => <MapAnalysisBatch key={batch.batchIndex} batch={batch} />} />}
    </div>
  )
}

function MapAnalysisBatch({ batch }: { batch: GlobalMapBatchView }): React.ReactElement {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50">
      <CollapsibleTrigger asChild>
        <button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={t(open ? "explainability.actions.collapseMapAnalysisBatch" : "explainability.actions.expandMapAnalysisBatch", { batch: batch.batchIndex + 1 })}>
          <span className="font-medium">{t("explainability.labels.batchValue", { value: batch.batchIndex + 1 })}</span>
          <Badge variant="outline">{batchStatusLabel(t, batch)}</Badge>
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent className="min-w-0 space-y-4 border-t p-3">
        <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.labels.reports" value={batch.reportCount} /><Metric label="explainability.labels.model" value={batch.model ?? t("explainability.labels.notAvailableYet")} />{batch.inputTokens === undefined ? null : <Metric label="explainability.labels.inputTokens" value={batch.inputTokens.toLocaleString()} />}{batch.outputTokens === undefined ? null : <Metric label="explainability.labels.outputTokens" value={batch.outputTokens.toLocaleString()} />}{batch.elapsedMs === undefined ? null : <Metric label="explainability.labels.latency" value={`${batch.elapsedMs.toLocaleString()} ms`} />}</dl>
        <div className="flex min-w-0 flex-wrap gap-2">
          <CapturedContentViewer buttonLabel="explainability.actions.viewMapContext" title={t("explainability.labels.mapContextTitle", { batch: batch.batchIndex + 1 })} content={batch.exactContext} unavailableMessage="explainability.messages.mapContextNotCaptured" testId={`analysis-exact-map-context-${batch.batchIndex}`} />
          <CapturedContentViewer buttonLabel="explainability.actions.viewMapPrompt" title={t("explainability.labels.mapPromptTitle", { batch: batch.batchIndex + 1 })} content={batch.exactPrompt} unavailableMessage="explainability.messages.mapPromptNotCaptured" testId={`exact-map-prompt-${batch.batchIndex}`} />
          <CapturedContentViewer buttonLabel="explainability.actions.viewRawMapResponse" title={t("explainability.labels.mapRawResponseTitle", { batch: batch.batchIndex + 1 })} content={batch.rawResponse} unavailableMessage="explainability.messages.mapRawResponseNotCaptured" testId={`raw-map-response-${batch.batchIndex}`} preview={false} />
        </div>
        <section aria-label={t("explainability.labels.parsedMapBatchPoints", { batch: batch.batchIndex + 1 })}><h4 className="text-xs font-semibold">{t("explainability.labels.parsedPoints")}</h4><p className="text-[10px] text-muted-foreground">{t("explainability.messages.parsedMapPointNotice")}</p><MapPointList points={batch.points} /></section>
      </CollapsibleContent>
    </Collapsible>
  )
}

function MapPointList({ points }: { points: GlobalMapPointView[] }): React.ReactElement {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? points : points.slice(0, INITIAL_POINTS)
  if (points.length === 0) return <p className="mt-2 text-xs text-muted-foreground">{t("explainability.messages.noParsedPointsAvailableYet")}</p>
  return (
    <div className="mt-2 space-y-2">
      <div className="divide-y rounded border bg-muted/10">{visible.map((point) => <div key={point.identity} className="min-w-0 px-2 py-2 text-xs"><div className="flex items-center justify-between gap-2"><span className="font-medium">{t("explainability.labels.pointValue", { value: point.point_index })}</span><Badge variant="outline">{t("explainability.labels.scoreValue", { value: point.score })}</Badge></div><p className="mt-1 min-w-0 whitespace-pre-wrap break-words text-muted-foreground">{point.answer ?? t("explainability.messages.mapPointAnswerNotCaptured")}</p></div>)}</div>
      {points.length > INITIAL_POINTS ? <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)}>{showAll ? t("explainability.actions.showFewerPoints") : t("explainability.counts.showAllCountPoints", { count: points.length })}</Button> : null}
    </div>
  )
}

function EvidenceReductionContent({ summary }: { summary: Extract<GlobalSemanticStep, { kind: "evidence-reduction" }>["summary"] }): React.ReactElement {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? summary.decisions : summary.decisions.slice(0, INITIAL_POINTS)
  if (!summary.built) return <p className="mt-3 text-xs text-muted-foreground">{t("explainability.messages.reduceDecisionsAreNotAvailableYet")}</p>
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.labels.candidatePoints" value={summary.candidatePointCount ?? 0} /><Metric label="explainability.labels.positive" value={summary.positivePointCount ?? 0} /><Metric label="graph.labels.included" value={summary.selectedPointCount ?? 0} /><Metric label="explainability.labels.nonPositive" value={summary.nonPositiveCount ?? 0} /><Metric label="explainability.labels.tokenBudgetExcluded" value={summary.tokenBudgetExcludedCount ?? 0} />{summary.tokensUsed === undefined ? null : <Metric label="answer.labels.tokens" value={`${summary.tokensUsed.toLocaleString()}${summary.tokenBudget === undefined ? "" : ` / ${summary.tokenBudget.toLocaleString()}`}`} />}</dl>
      {summary.skippedReason === "no_positive_points" ? <p className="rounded border bg-muted/20 p-3 text-xs font-medium">{t("explainability.labels.reduceSkippedNoPositivePoints")}</p> : null}
      {visible.length === 0 ? <p className="text-xs text-muted-foreground">{t("explainability.messages.waitingForReduceDecisions")}</p> : <div className="divide-y rounded border bg-background/50">{visible.map((decision) => <ReduceDecisionRow key={decision.identity} decision={decision} />)}</div>}
      {summary.decisions.length > INITIAL_POINTS ? <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)}>{showAll ? t("explainability.actions.showFewerPoints") : t("explainability.counts.showAllCountDecisions", { count: summary.decisions.length })}</Button> : null}
      <CapturedContentViewer buttonLabel="explainability.actions.viewReduceContext" title={t("explainability.labels.reduceContext")} content={summary.exactContext} unavailableMessage="explainability.messages.reduceContextNotCaptured" testId="exact-reduce-context" />
    </div>
  )
}

function ReduceDecisionRow({ decision }: { decision: GlobalReduceDecisionView }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 overflow-hidden px-2 py-2 text-xs">
      <span className="pt-0.5"><ReduceDecisionIcon decision={decision} /></span>
      <div className="min-w-0"><p className="font-medium">{t("explainability.labels.reducePoint", { batch: decision.batch_index + 1, point: decision.point_index, score: decision.score })}</p><p className="mt-1 whitespace-pre-wrap break-words text-muted-foreground">{decision.answer ?? t("explainability.messages.mapPointAnswerNotCaptured")}</p></div>
      <Badge variant="outline" className="shrink-0">{t(reduceDecisionLabel(decision))}</Badge>
    </div>
  )
}

function AnswerGenerationContent({ summary }: { summary: Extract<GlobalSemanticStep, { kind: "global-answer-generation" }>["summary"] }): React.ReactElement {
  const { t } = useTranslation()
  if (summary.noDataAnswerReturned) return <p className="mt-3 rounded border bg-muted/20 p-3 text-xs">{t("explainability.messages.noDataAnswerReturnedTheReduceLlmWasNotInvoked")}</p>
  if (summary.noDataPathSelected) return <p className="mt-3 rounded border bg-muted/20 p-3 text-xs">{t("explainability.messages.noDataPathSelectedTheReduceLlmWasSkipped")}</p>
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="answer.labels.calls" value={summary.calls} /><Metric label="explainability.labels.status" value={t(summary.generated ? "explainability.labels.answerGenerated" : summary.calls === 0 ? "explainability.labels.waiting" : "explainability.labels.generating")} />{summary.model === undefined ? null : <Metric label="explainability.labels.model" value={summary.model} />}{summary.inputTokens === undefined ? null : <Metric label="explainability.labels.inputTokens" value={summary.inputTokens.toLocaleString()} />}{summary.outputTokens === undefined ? null : <Metric label="explainability.labels.outputTokens" value={summary.outputTokens.toLocaleString()} />}{summary.elapsedMs === undefined ? null : <Metric label="explainability.labels.latency" value={`${summary.elapsedMs.toLocaleString()} ms`} />}</dl>
      <div className="flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewReducePrompt" title={t("explainability.labels.reducePrompt")} content={summary.exactPrompt} unavailableMessage="explainability.messages.reducePromptNotCaptured" testId="exact-reduce-prompt" /><CapturedContentViewer buttonLabel="explainability.actions.viewRawReduceResponse" title={t("explainability.labels.rawReduceResponse")} content={summary.rawResponse} unavailableMessage="explainability.messages.reduceRawResponseNotCaptured" testId="raw-reduce-response" preview={false} /></div>
    </div>
  )
}

function LimitedBatchList({ batches, renderBatch }: { batches: GlobalMapBatchView[]; renderBatch: (batch: GlobalMapBatchView) => React.ReactNode }): React.ReactElement {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? batches : batches.slice(0, INITIAL_BATCHES)
  return <div className="space-y-2"><div className="space-y-2">{visible.map(renderBatch)}</div>{batches.length > INITIAL_BATCHES ? <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)}>{showAll ? t("explainability.actions.showFewerBatches") : t("explainability.counts.showAllCountBatches", { count: batches.length })}</Button> : null}</div>
}

function Metric({ label, value }: { label: StudioTranslationKey; value: string | number }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="min-w-0 rounded border bg-muted/20 px-2 py-1.5"><dt className="text-muted-foreground">{t(label)}</dt><dd className="mt-0.5 break-words font-medium">{value}</dd></div>
}

function batchStatusLabel(t: TFunction, batch: GlobalMapBatchView): string {
  if (batch.status === "ready") return t("explainability.labels.ready")
  if (batch.status === "analyzing") return t("explainability.labels.analyzing")
  if (batch.status === "response_received") return t("explainability.labels.responseReceived")
  return t("explainability.counts.completedCountPoint", { count: batch.points.length })
}

function reduceDecisionLabel(decision: GlobalReduceDecisionView): StudioTranslationKey {
  if (decision.reason === "selected") return "graph.labels.included"
  if (decision.reason === "non_positive_score") return "explainability.labels.nonPositive"
  return "explainability.labels.tokenBudget"
}

function waveSourceLabel(source: DynamicTraversalWaveView["source"]): StudioTranslationKey {
  if (source === "initial") return "explainability.labels.initial"
  if (source === "child_expansion") return "explainability.labels.childExpansion"
  return "explainability.labels.fallback"
}

function dynamicDecisionLabel(decision: DynamicCommunityDecisionView): StudioTranslationKey {
  if (decision.selected) return "explainability.actions.selected"
  if (decision.threshold_passed) return "explainability.labels.passedThresholdNotRetained"
  return "explainability.labels.belowThreshold"
}

function ReduceDecisionIcon({ decision }: { decision: GlobalReduceDecisionView }): React.ReactElement {
  const { t } = useTranslation()
  if (decision.reason === "selected") return <Check className="size-3.5 text-success" aria-label={t("graph.labels.included")} />
  if (decision.reason === "non_positive_score") return <X className="size-3.5 text-muted-foreground" aria-label={t("explainability.labels.nonPositive")} />
  return <Circle className="size-3.5 text-muted-foreground" aria-label={t("explainability.labels.tokenBudget")} />
}
