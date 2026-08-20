import { useState } from "react"
import { Check, Circle, Files, Filter, GitBranch, Sparkles, Split, X } from "lucide-react"

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

interface GlobalSemanticStepCardProps {
  step: GlobalSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

const INITIAL_BATCHES = 6
const INITIAL_POINTS = 20
const INITIAL_COMMUNITIES = 20
const INITIAL_WAVES = 5
const WAVE_ID_PAGE_SIZE = 20

export function GlobalSemanticStepCard({ step, onFocusGraph }: GlobalSemanticStepCardProps): React.ReactElement {
  return (
    <article className="min-w-0 rounded-md border bg-card/70 p-3" aria-label={step.title}>
      <div className="flex min-w-0 items-start gap-2">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><GlobalStepIcon kind={step.kind} /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{step.title}</h3><GlobalStepSummary step={step} /></div>
      </div>
      <GlobalStepContent step={step} />
      <TechnicalDetails rawEvents={step.rawEvents} onFocusGraph={onFocusGraph} />
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
  if (step.kind === "community-selection") {
    const summary = step.summary
    if (summary.completed) return <p className="mt-1 text-xs text-muted-foreground">{summary.visitedCount ?? 0} communities rated · {summary.selectedCount ?? 0} selected{summary.threshold === undefined ? "" : ` · threshold ${summary.threshold}`}{summary.numRepeats === undefined ? "" : ` · ${summary.numRepeats} ${summary.numRepeats === 1 ? "repeat" : "repeats"}`}</p>
    if (summary.activeWave !== undefined) return <p className="mt-1 text-xs text-muted-foreground">Rating wave {summary.activeWave + 1} · {summary.attemptsCompleted} / {summary.attemptsStarted} attempts completed</p>
    return <p className="mt-1 text-xs text-muted-foreground">Selecting communities</p>
  }
  if (step.kind === "community-context") {
    const summary = step.summary
    if (!summary.built) return <p className="mt-1 text-xs text-muted-foreground">Waiting for community context</p>
    const batchProgress = summary.batchCount === undefined || summary.batches.length === summary.batchCount
      ? `${summary.batchCount ?? summary.batches.length} map batches · ${summary.tokensUsed.toLocaleString()} tokens`
      : `${summary.batches.length} / ${summary.batchCount} batch contexts ready`
    return <p className="mt-1 text-xs text-muted-foreground">{summary.reportCount ?? 0} community reports · {batchProgress}</p>
  }
  if (step.kind === "map-analysis") {
    const summary = step.summary
    if (!summary.started) return <p className="mt-1 text-xs text-muted-foreground">Waiting for Map analysis</p>
    return <p className="mt-1 text-xs text-muted-foreground">{summary.batchCount ?? 0} batches · {summary.analystCalls} analyst calls · {summary.pointCount} points · {summary.positivePointCount} positive</p>
  }
  if (step.kind === "evidence-reduction") {
    const summary = step.summary
    if (!summary.built) return <p className="mt-1 text-xs text-muted-foreground">Waiting for evidence reduction</p>
    if (summary.skippedReason === "no_positive_points") return <p className="mt-1 text-xs text-muted-foreground">{summary.candidatePointCount ?? 0} candidate points · no positive evidence · Reduce LLM skipped</p>
    const tokens = summary.tokensUsed === undefined ? "" : ` · ${summary.tokensUsed.toLocaleString()}${summary.tokenBudget === undefined ? "" : ` / ${summary.tokenBudget.toLocaleString()}`} tokens`
    return <p className="mt-1 text-xs text-muted-foreground">{summary.candidatePointCount ?? 0} candidates · {summary.positivePointCount ?? 0} positive · {summary.selectedPointCount ?? 0} included{tokens}{summary.truncated ? " · truncated" : ""}</p>
  }
  const summary = step.summary
  if (summary.noDataAnswerReturned) return <p className="mt-1 text-xs text-muted-foreground">No-data answer returned · Reduce LLM not invoked</p>
  if (summary.noDataPathSelected) return <p className="mt-1 text-xs text-muted-foreground">No-data path selected · Reduce LLM skipped</p>
  if (summary.generated) return <p className="mt-1 text-xs text-muted-foreground">Answer generated · {summary.inputTokens?.toLocaleString() ?? 0} input · {summary.outputTokens?.toLocaleString() ?? 0} output</p>
  return <p className="mt-1 text-xs text-muted-foreground">{summary.calls === 0 ? "Waiting for Reduce LLM" : "Reduce LLM running"}</p>
}

function GlobalStepContent({ step }: { step: GlobalSemanticStep }): React.ReactNode {
  if (step.kind === "community-selection") return <CommunitySelectionContent summary={step.summary} />
  if (step.kind === "community-context") return <CommunityContextContent batches={step.summary.batches} />
  if (step.kind === "map-analysis") return <MapAnalysisContent batches={step.summary.batches} />
  if (step.kind === "evidence-reduction") return <EvidenceReductionContent summary={step.summary} />
  return <AnswerGenerationContent summary={step.summary} />
}

function CommunitySelectionContent({ summary }: { summary: DynamicCommunitySelectionSummary }): React.ReactElement {
  const [showAllCommunities, setShowAllCommunities] = useState(false)
  const [showAllWaves, setShowAllWaves] = useState(false)
  const visibleCommunities = showAllCommunities ? summary.decisions : summary.decisions.slice(0, INITIAL_COMMUNITIES)
  const visibleWaves = showAllWaves ? summary.waves : summary.waves.slice(0, INITIAL_WAVES)
  return (
    <div className="mt-3 min-w-0 space-y-4">
      <dl className="grid grid-cols-2 gap-2 text-xs">
        {summary.initialCommunityCount === undefined ? null : <Metric label="Initial communities" value={summary.initialCommunityCount} />}
        {summary.threshold === undefined ? null : <Metric label="Threshold" value={summary.threshold} />}
        {summary.numRepeats === undefined ? null : <Metric label="Repeats" value={summary.numRepeats} />}
        {summary.maxLevel === undefined ? null : <Metric label="Fallback max level" value={summary.maxLevel} />}
        {summary.visitedCount === undefined ? null : <Metric label="Communities rated" value={summary.visitedCount} />}
        {summary.thresholdPassedCount === undefined ? null : <Metric label="Passed threshold" value={summary.thresholdPassedCount} />}
        {summary.selectedCount === undefined ? null : <Metric label="Selected" value={summary.selectedCount} />}
        <Metric label="Rating attempts" value={`${summary.attemptsCompleted} / ${summary.attemptsStarted} responses`} />
      </dl>
      <section aria-label="Dynamic traversal waves" className="min-w-0"><h4 className="text-xs font-semibold">Traversal waves</h4><div className="mt-2 space-y-2">{visibleWaves.map((wave) => <TraversalWaveRow key={`${wave.waveIndex}:${wave.source}`} wave={wave} />)}</div>{summary.waves.length > INITIAL_WAVES ? <Button variant="ghost" size="sm" onClick={() => setShowAllWaves((value) => !value)}>{showAllWaves ? "Show fewer waves" : `Show all ${summary.waves.length} waves`}</Button> : null}</section>
      <section aria-label="Dynamic community decisions" className="min-w-0"><h4 className="text-xs font-semibold">Community decisions</h4>{summary.completed ? <><div className="mt-2 space-y-2">{visibleCommunities.map((decision) => <CommunityDecisionRow key={decision.community_id} decision={decision} />)}</div>{summary.decisions.length > INITIAL_COMMUNITIES ? <Button variant="ghost" size="sm" onClick={() => setShowAllCommunities((value) => !value)}>{showAllCommunities ? "Show fewer communities" : `Show all ${summary.decisions.length} communities`}</Button> : null}</> : <p className="mt-2 text-xs text-muted-foreground">Final community decisions are available after selection completes.</p>}</section>
    </div>
  )
}

function TraversalWaveRow({ wave }: { wave: DynamicTraversalWaveView }): React.ReactElement {
  const [open, setOpen] = useState(false)
  const [visibleIdCount, setVisibleIdCount] = useState(WAVE_ID_PAGE_SIZE)
  const visibleIds = wave.communityIds.slice(0, visibleIdCount)
  const remainingIdCount = wave.communityIds.length - visibleIds.length
  return <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50"><CollapsibleTrigger asChild><button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={`${open ? "Hide" : "Show"} community IDs for wave ${wave.waveIndex + 1}`}><span className="font-medium">Wave {wave.waveIndex + 1} · {waveSourceLabel(wave.source)}</span><span className="shrink-0 text-muted-foreground">{wave.communityIds.length} communities</span></button></CollapsibleTrigger><CollapsibleContent className="border-t p-3"><ul className="max-h-32 overflow-auto rounded border bg-muted/20 p-2 font-mono text-[11px]">{visibleIds.map((id) => <li key={id} className="break-all">{id}</li>)}</ul><div className="mt-1 flex flex-wrap gap-1">{remainingIdCount > 0 ? <Button variant="ghost" size="sm" onClick={() => setVisibleIdCount((count) => Math.min(count + WAVE_ID_PAGE_SIZE, wave.communityIds.length))}>Show next {Math.min(WAVE_ID_PAGE_SIZE, remainingIdCount)} community IDs</Button> : null}{visibleIdCount > WAVE_ID_PAGE_SIZE ? <Button variant="ghost" size="sm" onClick={() => setVisibleIdCount(WAVE_ID_PAGE_SIZE)}>Show fewer community IDs</Button> : null}</div></CollapsibleContent></Collapsible>
}

function CommunityDecisionRow({ decision }: { decision: DynamicCommunityDecisionView }): React.ReactElement {
  const [open, setOpen] = useState(false)
  const status = dynamicDecisionLabel(decision)
  return <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50"><CollapsibleTrigger asChild><button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={`${open ? "Collapse" : "Expand"} rating details for community ${decision.community_id}`}><span className="min-w-0"><span className="block truncate font-medium">Community {decision.community_id}</span><span className="text-muted-foreground">Level {decision.level} · Majority rating {decision.selected_rating}</span></span><Badge variant="outline" className="shrink-0">{status}</Badge></button></CollapsibleTrigger><CollapsibleContent className="min-w-0 space-y-3 border-t p-3"><p className="font-mono text-[10px] text-muted-foreground break-all">report_id {decision.report_id}</p><h5 className="text-xs font-semibold">Rating attempts</h5>{decision.attempts.length === 0 ? <p className="text-xs text-muted-foreground">Rating attempt details are not available.</p> : <div className="space-y-2">{decision.attempts.map((attempt) => <RatingAttemptRow key={attempt.identity} attempt={attempt} />)}</div>}</CollapsibleContent></Collapsible>
}

function RatingAttemptRow({ attempt }: { attempt: DynamicRatingAttemptView }): React.ReactElement {
  return <div className="min-w-0 space-y-2 rounded border bg-muted/10 p-2 text-xs"><div className="flex min-w-0 items-center justify-between gap-2"><span className="font-medium">Repeat {attempt.repeatIndex + 1}</span><Badge variant="outline">{attempt.status === "response_received" ? "Response received" : "Rating"}</Badge></div><dl className="grid grid-cols-2 gap-2"><Metric label="Model" value={attempt.model ?? "Not available yet"} />{attempt.inputTokens === undefined ? null : <Metric label="Input tokens" value={attempt.inputTokens.toLocaleString()} />}{attempt.outputTokens === undefined ? null : <Metric label="Output tokens" value={attempt.outputTokens.toLocaleString()} />}{attempt.elapsedMs === undefined ? null : <Metric label="Latency" value={`${attempt.elapsedMs.toLocaleString()} ms`} />}</dl><div className="flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="View Rating Prompt" title={`Community ${attempt.communityId} · Repeat ${attempt.repeatIndex + 1} Prompt`} content={attempt.exactPrompt} unavailableMessage="Rating prompt content was not captured. Run with Content or Debug to inspect the exact rendered prompt." testId={`rating-prompt-${attempt.communityId}-${attempt.repeatIndex}`} /><CapturedContentViewer buttonLabel="View Raw Rating Response" title={`Community ${attempt.communityId} · Repeat ${attempt.repeatIndex + 1} Raw Response`} content={attempt.rawResponse} unavailableMessage="Raw rating response content was not captured. Run with Content or Debug to inspect the provider response." testId={`rating-response-${attempt.communityId}-${attempt.repeatIndex}`} preview={false} /></div></div>
}

function CommunityContextContent({ batches }: { batches: GlobalMapBatchView[] }): React.ReactElement {
  return (
    <div className="mt-3 min-w-0">
      {batches.length === 0 ? <p className="text-xs text-muted-foreground">Waiting for community context batches.</p> : <LimitedBatchList batches={batches} renderBatch={(batch) => <CommunityBatch key={batch.batchIndex} batch={batch} />} />}
    </div>
  )
}

function CommunityBatch({ batch }: { batch: GlobalMapBatchView }): React.ReactElement {
  const [open, setOpen] = useState(false)
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50">
      <CollapsibleTrigger asChild>
        <button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={`${open ? "Collapse" : "Expand"} map batch ${batch.batchIndex + 1}`}>
          <span className="font-medium">Batch {batch.batchIndex + 1}</span>
          <span className="shrink-0 text-muted-foreground">{batch.reportCount} reports · {batch.tokensUsed.toLocaleString()} / {batch.tokenBudget.toLocaleString()} tokens</span>
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent className="min-w-0 space-y-3 border-t p-3">
        <p className="font-mono text-[10px] text-muted-foreground">batch_index {batch.batchIndex}</p>
        <div><p className="text-[11px] font-semibold text-muted-foreground uppercase">Stable report IDs</p><ul className="mt-1 max-h-32 overflow-auto rounded border bg-muted/20 p-2 font-mono text-[11px]">{batch.reportIds.map((id) => <li key={id} className="break-all">{id}</li>)}</ul></div>
        <CapturedContentViewer buttonLabel="View Map Context" title={`Map Batch ${batch.batchIndex + 1} Context`} content={batch.exactContext} unavailableMessage="Map context content was not captured. Run with Content or Debug to inspect exact input." testId={`exact-map-context-${batch.batchIndex}`} />
      </CollapsibleContent>
    </Collapsible>
  )
}

function MapAnalysisContent({ batches }: { batches: GlobalMapBatchView[] }): React.ReactElement {
  return (
    <div className="mt-3 min-w-0">
      {batches.length === 0 ? <p className="text-xs text-muted-foreground">Waiting for Map analysis.</p> : <LimitedBatchList batches={batches} renderBatch={(batch) => <MapAnalysisBatch key={batch.batchIndex} batch={batch} />} />}
    </div>
  )
}

function MapAnalysisBatch({ batch }: { batch: GlobalMapBatchView }): React.ReactElement {
  const [open, setOpen] = useState(false)
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="min-w-0 rounded border bg-background/50">
      <CollapsibleTrigger asChild>
        <button type="button" className="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-expanded={open} aria-label={`${open ? "Collapse" : "Expand"} Map analysis batch ${batch.batchIndex + 1}`}>
          <span className="font-medium">Batch {batch.batchIndex + 1}</span>
          <Badge variant="outline">{batchStatusLabel(batch)}</Badge>
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent className="min-w-0 space-y-4 border-t p-3">
        <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="Reports" value={batch.reportCount} /><Metric label="Model" value={batch.model ?? "Not available yet"} />{batch.inputTokens === undefined ? null : <Metric label="Input tokens" value={batch.inputTokens.toLocaleString()} />}{batch.outputTokens === undefined ? null : <Metric label="Output tokens" value={batch.outputTokens.toLocaleString()} />}{batch.elapsedMs === undefined ? null : <Metric label="Latency" value={`${batch.elapsedMs.toLocaleString()} ms`} />}</dl>
        <div className="flex min-w-0 flex-wrap gap-2">
          <CapturedContentViewer buttonLabel="View Map Context" title={`Map Batch ${batch.batchIndex + 1} Context`} content={batch.exactContext} unavailableMessage="Map context content was not captured. Run with Content or Debug to inspect exact input." testId={`analysis-exact-map-context-${batch.batchIndex}`} />
          <CapturedContentViewer buttonLabel="View Map Prompt" title={`Map Batch ${batch.batchIndex + 1} Prompt`} content={batch.exactPrompt} unavailableMessage="Map prompt content was not captured. Run with Content or Debug to inspect the exact rendered prompt." testId={`exact-map-prompt-${batch.batchIndex}`} />
          <CapturedContentViewer buttonLabel="View Raw Map Response" title={`Map Batch ${batch.batchIndex + 1} Raw Response`} content={batch.rawResponse} unavailableMessage="Raw Map response content was not captured. Run with Content or Debug to inspect the provider response." testId={`raw-map-response-${batch.batchIndex}`} preview={false} />
        </div>
        <section aria-label={`Parsed points for map batch ${batch.batchIndex + 1}`}><h4 className="text-xs font-semibold">Parsed points</h4><p className="text-[10px] text-muted-foreground">Parsed from the raw provider response; production here does not imply Reduce inclusion.</p><MapPointList points={batch.points} /></section>
      </CollapsibleContent>
    </Collapsible>
  )
}

function MapPointList({ points }: { points: GlobalMapPointView[] }): React.ReactElement {
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? points : points.slice(0, INITIAL_POINTS)
  if (points.length === 0) return <p className="mt-2 text-xs text-muted-foreground">No parsed points available yet.</p>
  return (
    <div className="mt-2 space-y-2">
      <div className="divide-y rounded border bg-muted/10">{visible.map((point) => <div key={point.identity} className="min-w-0 px-2 py-2 text-xs"><div className="flex items-center justify-between gap-2"><span className="font-medium">Point {point.point_index}</span><Badge variant="outline">Score {point.score}</Badge></div><p className="mt-1 min-w-0 whitespace-pre-wrap break-words text-muted-foreground">{point.answer ?? "Point answer was not captured. Run with Content or Debug to inspect it."}</p></div>)}</div>
      {points.length > INITIAL_POINTS ? <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)}>{showAll ? "Show fewer points" : `Show all ${points.length} points`}</Button> : null}
    </div>
  )
}

function EvidenceReductionContent({ summary }: { summary: Extract<GlobalSemanticStep, { kind: "evidence-reduction" }>["summary"] }): React.ReactElement {
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? summary.decisions : summary.decisions.slice(0, INITIAL_POINTS)
  if (!summary.built) return <p className="mt-3 text-xs text-muted-foreground">Reduce decisions are not available yet.</p>
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="Candidate points" value={summary.candidatePointCount ?? 0} /><Metric label="Positive" value={summary.positivePointCount ?? 0} /><Metric label="Included" value={summary.selectedPointCount ?? 0} /><Metric label="Non-positive" value={summary.nonPositiveCount ?? 0} /><Metric label="Token-budget excluded" value={summary.tokenBudgetExcludedCount ?? 0} />{summary.tokensUsed === undefined ? null : <Metric label="Tokens" value={`${summary.tokensUsed.toLocaleString()}${summary.tokenBudget === undefined ? "" : ` / ${summary.tokenBudget.toLocaleString()}`}`} />}</dl>
      {summary.skippedReason === "no_positive_points" ? <p className="rounded border bg-muted/20 p-3 text-xs font-medium">Reduce skipped — no positive points</p> : null}
      {visible.length === 0 ? <p className="text-xs text-muted-foreground">Waiting for Reduce decisions.</p> : <div className="divide-y rounded border bg-background/50">{visible.map((decision) => <ReduceDecisionRow key={decision.identity} decision={decision} />)}</div>}
      {summary.decisions.length > INITIAL_POINTS ? <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)}>{showAll ? "Show fewer points" : `Show all ${summary.decisions.length} decisions`}</Button> : null}
      <CapturedContentViewer buttonLabel="View Reduce Context" title="Reduce Context" content={summary.exactContext} unavailableMessage="Reduce context content was not captured. Run with Content or Debug to inspect exact input." testId="exact-reduce-context" />
    </div>
  )
}

function ReduceDecisionRow({ decision }: { decision: GlobalReduceDecisionView }): React.ReactElement {
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 overflow-hidden px-2 py-2 text-xs">
      <span className="pt-0.5"><ReduceDecisionIcon decision={decision} /></span>
      <div className="min-w-0"><p className="font-medium">Batch {decision.batch_index + 1} · Point {decision.point_index} · Score {decision.score}</p><p className="mt-1 whitespace-pre-wrap break-words text-muted-foreground">{decision.answer ?? "Point answer was not captured. Run with Content or Debug to inspect it."}</p></div>
      <Badge variant="outline" className="shrink-0">{reduceDecisionLabel(decision)}</Badge>
    </div>
  )
}

function AnswerGenerationContent({ summary }: { summary: Extract<GlobalSemanticStep, { kind: "global-answer-generation" }>["summary"] }): React.ReactElement {
  if (summary.noDataAnswerReturned) return <p className="mt-3 rounded border bg-muted/20 p-3 text-xs">No-data answer returned. The Reduce LLM was not invoked.</p>
  if (summary.noDataPathSelected) return <p className="mt-3 rounded border bg-muted/20 p-3 text-xs">No-data path selected. The Reduce LLM was skipped.</p>
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <dl className="grid grid-cols-2 gap-2 text-xs"><Metric label="Calls" value={summary.calls} /><Metric label="Status" value={summary.generated ? "Answer generated" : summary.calls === 0 ? "Waiting" : "Generating"} />{summary.model === undefined ? null : <Metric label="Model" value={summary.model} />}{summary.inputTokens === undefined ? null : <Metric label="Input tokens" value={summary.inputTokens.toLocaleString()} />}{summary.outputTokens === undefined ? null : <Metric label="Output tokens" value={summary.outputTokens.toLocaleString()} />}{summary.elapsedMs === undefined ? null : <Metric label="Latency" value={`${summary.elapsedMs.toLocaleString()} ms`} />}</dl>
      <div className="flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="View Reduce Prompt" title="Reduce Prompt" content={summary.exactPrompt} unavailableMessage="Reduce prompt content was not captured. Run with Content or Debug to inspect the exact rendered prompt." testId="exact-reduce-prompt" /><CapturedContentViewer buttonLabel="View Raw Reduce Response" title="Raw Reduce Response" content={summary.rawResponse} unavailableMessage="Raw Reduce response content was not captured. Run with Content or Debug to inspect the provider response." testId="raw-reduce-response" preview={false} /></div>
    </div>
  )
}

function LimitedBatchList({ batches, renderBatch }: { batches: GlobalMapBatchView[]; renderBatch: (batch: GlobalMapBatchView) => React.ReactNode }): React.ReactElement {
  const [showAll, setShowAll] = useState(false)
  const visible = showAll ? batches : batches.slice(0, INITIAL_BATCHES)
  return <div className="space-y-2"><div className="space-y-2">{visible.map(renderBatch)}</div>{batches.length > INITIAL_BATCHES ? <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)}>{showAll ? "Show fewer batches" : `Show all ${batches.length} batches`}</Button> : null}</div>
}

function Metric({ label, value }: { label: string; value: string | number }): React.ReactElement {
  return <div className="min-w-0 rounded border bg-muted/20 px-2 py-1.5"><dt className="text-muted-foreground">{label}</dt><dd className="mt-0.5 break-words font-medium">{value}</dd></div>
}

function batchStatusLabel(batch: GlobalMapBatchView): string {
  if (batch.status === "ready") return "Ready"
  if (batch.status === "analyzing") return "Analyzing"
  if (batch.status === "response_received") return "Response received"
  return `Completed · ${batch.points.length} ${batch.points.length === 1 ? "point" : "points"}`
}

function reduceDecisionLabel(decision: GlobalReduceDecisionView): string {
  if (decision.reason === "selected") return "Included"
  if (decision.reason === "non_positive_score") return "Non-positive"
  return "Token budget"
}

function waveSourceLabel(source: DynamicTraversalWaveView["source"]): string {
  if (source === "initial") return "Initial"
  if (source === "child_expansion") return "Child expansion"
  return "Fallback"
}

function dynamicDecisionLabel(decision: DynamicCommunityDecisionView): string {
  if (decision.selected) return "Selected"
  if (decision.threshold_passed) return "Passed threshold · not retained"
  return "Below threshold"
}

function ReduceDecisionIcon({ decision }: { decision: GlobalReduceDecisionView }): React.ReactElement {
  if (decision.reason === "selected") return <Check className="size-3.5 text-success" aria-label="Included" />
  if (decision.reason === "non_positive_score") return <X className="size-3.5 text-muted-foreground" aria-label="Non-positive" />
  return <Circle className="size-3.5 text-muted-foreground" aria-label="Token budget" />
}
