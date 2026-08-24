import { useState } from "react"
import type { TFunction } from "i18next"
import { BrainCircuit, ChevronDown, ChevronRight, GitFork, Sparkles } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityEnvelope } from "@/api/types"
import { CapturedContentViewer } from "@/components/explainability/captured-content-viewer"
import { TechnicalDetails } from "@/components/explainability/technical-details"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { DriftActionAttemptView, DriftActionNodeView, DriftPrimerFoldView, DriftSemanticStep } from "@/lib/semantic-drift"
import { semanticStepTitle } from "@/i18n/presentation"
import type { StudioTranslationKey } from "@/i18n/types"

interface DriftSemanticStepCardProps {
  step: DriftSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  showDeveloperDetails: boolean
}

const INITIAL_ACTIONS = 20
const INITIAL_EDGES = 30
const INITIAL_FOLDS = 12

export function DriftSemanticStepCard({ step, onFocusGraph, showDeveloperDetails }: DriftSemanticStepCardProps): React.ReactElement {
  const { t } = useTranslation()
  const title = semanticStepTitle(t, step.kind)
  return (
    <article className="min-w-0 rounded-md border bg-card/70 p-3" aria-label={title}>
      <div className="flex min-w-0 gap-2">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><StepIcon kind={step.kind} /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{title}</h3><StepSummary step={step} /></div>
      </div>
      <StepContent step={step} />
      <TechnicalDetails visible={showDeveloperDetails} rawEvents={step.rawEvents} onFocusGraph={onFocusGraph} />
    </article>
  )
}

function StepIcon({ kind }: { kind: DriftSemanticStep["kind"] }): React.ReactElement {
  if (kind === "drift-primer-ranking") return <BrainCircuit className="size-4" />
  if (kind === "drift-exploration") return <GitFork className="size-4" />
  return <Sparkles className="size-4" />
}

function StepSummary({ step }: { step: DriftSemanticStep }): React.ReactElement {
  const { t } = useTranslation()
  if (step.kind === "drift-primer-ranking") {
    const summary = step.summary
    if (summary.aggregate === null) {
      const progress: StudioTranslationKey = summary.status === "hyde" ? "explainability.labels.generatingHydeExpansion" : summary.status === "embedding" ? "explainability.labels.embeddingExpandedQuery" : summary.status === "ranking" ? "explainability.labels.rankingCommunityReports" : "explainability.actions.runningPrimerFolds"
      return <p className="mt-1 text-xs text-muted-foreground">{t(progress)}</p>
    }
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.messages.primerSummary", { reports: summary.rankedReports.length, folds: summary.folds.length, followUps: summary.aggregate.followUpCount, score: summary.aggregate.score.toFixed(1) })}</p>
  }
  if (step.kind === "drift-exploration") {
    const summary = step.summary
    if (summary.activeDepth !== undefined && summary.nodeCount === undefined) {
      const selected = summary.depths.at(-1)?.selectedActionIds.length ?? 0
      return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.counts.depthActionsSelected", { depth: summary.activeDepth + 1, count: selected })}</p>
    }
    return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.messages.explorationSummary", { depths: summary.depths.length, attempts: summary.attempts.length, nodes: summary.nodeCount ?? summary.nodes.length, edges: summary.edgeCount ?? summary.edges.length })}</p>
  }
  const summary = step.summary
  if (!summary.built) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.waitingForFinalSynthesis")}</p>
  if (!summary.generated) return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.labels.generatingFinalSynthesis")}</p>
  return <p className="mt-1 text-xs text-muted-foreground">{t("explainability.messages.synthesisSummary", { answers: summary.includedAnswerCount ?? 0, nodes: summary.nodeCount ?? 0, edges: summary.edgeCount ?? 0 })}</p>
}

function StepContent({ step }: { step: DriftSemanticStep }): React.ReactNode {
  const { t } = useTranslation()
  if (step.kind === "drift-primer-ranking") return <PrimerContent summary={step.summary} />
  if (step.kind === "drift-exploration") return <ExplorationContent summary={step.summary} />
  const summary = step.summary
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.labels.reduceSelection")}>
        <h4 className="text-xs font-semibold">{t("explainability.labels.includedInReduce")}</h4>
        {summary.includedActionIds.length === 0 ? <p className="mt-1 text-xs text-muted-foreground">{t("explainability.messages.noIncludedAnswersRecordedYet")}</p> : <div className="mt-2 flex flex-wrap gap-1">{summary.includedActionIds.map((id, index) => <Badge key={`${id}:${index}`} variant="outline">#{id}</Badge>)}</div>}
      </section>
      <div className="flex min-w-0 flex-wrap gap-2">
        <CapturedContentViewer buttonLabel="explainability.actions.viewDriftState" title={t("explainability.labels.driftState")} content={summary.exactStateContext} unavailableMessage="explainability.messages.driftStateNotCaptured" testId="exact-drift-state" exactTabLabel="explainability.labels.exactSource" copyLabel="explainability.actions.copyExactDriftState" description="explainability.messages.driftStatePreviewNotice" />
        <CapturedContentViewer buttonLabel="explainability.actions.viewReduceContext" title={t("explainability.labels.reduceContext")} content={summary.exactReduceContext} unavailableMessage="explainability.messages.driftReduceContextNotCaptured" testId="exact-drift-reduce-context" exactTabLabel="explainability.labels.exactSource" copyLabel="explainability.actions.copyExactReduceContext" description="explainability.messages.reduceContextSourceNotice" />
        <CapturedContentViewer buttonLabel="explainability.actions.viewReducePrompt" title={t("explainability.labels.reducePrompt")} content={summary.exactPrompt} unavailableMessage="explainability.messages.reducePromptWasNotCapturedRunWithContentOrDebugMode" testId="exact-drift-reduce-prompt" />
        {summary.generated ? <CapturedContentViewer buttonLabel="explainability.actions.viewRawReduceResponse" title={t("explainability.labels.rawReduceResponse")} content={summary.rawResponse} unavailableMessage="explainability.messages.driftReduceRawResponseNotCaptured" testId="raw-drift-reduce-response" preview={false} /> : null}
      </div>
      <LlmMetrics view={summary} />
    </div>
  )
}

function PrimerContent({ summary }: { summary: Extract<DriftSemanticStep, { kind: "drift-primer-ranking" }>["summary"] }): React.ReactElement {
  const { t } = useTranslation()
  const [showAllFolds, setShowAllFolds] = useState(false)
  const [showAllReports, setShowAllReports] = useState(false)
  const reports = showAllReports ? summary.rankedReports : summary.rankedReports.slice(0, 20)
  const folds = showAllFolds ? summary.folds : summary.folds.slice(0, INITIAL_FOLDS)
  return (
    <div className="mt-3 min-w-0 space-y-3">
      {summary.hyde === null ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.labels.hydeExpansion")}><h4 className="text-xs font-semibold">{t("explainability.labels.hydeExpansion")}</h4><dl className="mt-2 grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.labels.template" value={t("explainability.labels.reportId", { id: summary.hyde.templateShortId })} /><Metric label="explainability.labels.stableId" value={summary.hyde.templateReportId} breakAll /><Metric label="explainability.labels.community" value={summary.hyde.templateCommunityId} breakAll /><Metric label="graph.actions.selection" value={`${summary.hyde.templateIndex + 1} / ${summary.hyde.reportCount}`} /></dl>{summary.hyde.completed ? <p className="mt-2 text-xs text-muted-foreground">{t(summary.hyde.usedOriginalQuery ? "explainability.labels.emptyExpansionOriginalQueryUsed" : "explainability.labels.hydeExpansionUsed")}</p> : <p className="mt-2 text-xs text-muted-foreground">{t("explainability.messages.generatingExpansion")}</p>}<div className="mt-2 flex flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewHydePrompt" title={t("explainability.labels.hydePrompt")} content={summary.hyde.exactPrompt} unavailableMessage="explainability.messages.hydePromptWasNotCapturedRunWithContentOrDebugMode" testId="exact-drift-hyde-prompt" />{summary.hyde.completed ? <CapturedContentViewer buttonLabel="explainability.actions.viewRawHydeResponse" title={t("explainability.labels.rawHydeResponse")} content={summary.hyde.rawResponse} unavailableMessage="explainability.messages.rawHydeResponseWasNotCapturedRunWithContentOrDebugMode" testId="raw-drift-hyde-response" preview={false} /> : null}</div>{summary.hyde.completed ? <div className="mt-2"><CapturedContentViewer buttonLabel="explainability.actions.viewEffectiveQuery" title={t("explainability.labels.effectiveEmbeddingQuery")} content={summary.hyde.effectiveQuery} unavailableMessage="explainability.messages.effectiveQueryNotCaptured" testId="exact-drift-effective-query" preview={false} /></div> : null}</section>}
      {summary.rankedReports.length === 0 ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.labels.rankedReports")}><h4 className="text-xs font-semibold">{t("explainability.labels.rankedReports")}</h4><div className="mt-2 divide-y overflow-hidden rounded border bg-background/50">{reports.map((report) => <div key={`${report.rank}:${report.report_id}`} className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 px-2 py-2 text-xs"><span className="font-mono">#{report.rank}</span><div className="min-w-0"><p className="truncate font-medium" title={report.short_id}>{t("explainability.labels.reportId", { id: report.short_id })}</p><p className="break-all font-mono text-[10px] text-muted-foreground">{report.report_id}</p><p className="break-all text-[10px] text-muted-foreground">{t("explainability.labels.communityId", { id: report.community_id })}</p></div><span className="font-mono">{report.similarity.toFixed(4)}</span></div>)}</div>{summary.rankedReports.length > 20 ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllReports} aria-label={showAllReports ? t("explainability.actions.showFewerRankedReports") : t("explainability.counts.showAllCountRankedReports", { count: summary.rankedReports.length })} onClick={() => setShowAllReports((value) => !value)}>{showAllReports ? t("explainability.actions.showFewer") : t("explainability.counts.showAllCount", { count: summary.rankedReports.length })}</Button> : null}</section>}
      {summary.folds.length === 0 ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.labels.primerFolds")}><h4 className="text-xs font-semibold">{t("explainability.labels.primerFolds")}</h4><div className="mt-2 space-y-2">{folds.map((fold) => <PrimerFold key={fold.foldIndex} fold={fold} />)}</div>{summary.folds.length > INITIAL_FOLDS ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllFolds} aria-label={showAllFolds ? t("explainability.actions.showFewerPrimerFolds") : t("explainability.counts.showAllCountPrimerFolds", { count: summary.folds.length })} onClick={() => setShowAllFolds((value) => !value)}>{showAllFolds ? t("explainability.actions.showFewer") : t("explainability.counts.showAllCount", { count: summary.folds.length })}</Button> : null}</section>}
      {summary.aggregate === null ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.labels.primerAggregate")}><h4 className="text-xs font-semibold">{t("explainability.labels.primerAggregate")}</h4><dl className="mt-2 grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.labels.score" value={summary.aggregate.score.toFixed(1)} /><Metric label="explainability.labels.followUps" value={summary.aggregate.followUpCount} /><Metric label="explainability.labels.rootAction" value={`#${summary.aggregate.rootActionId}`} /><Metric label="explainability.labels.targetIds" value={summary.aggregate.followUpActionIds.map((id) => `#${id}`).join(" ") || t("graph.labels.none")} /></dl><div className="mt-2 flex flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewPrimerAnswer" title={t("explainability.labels.primerIntermediateAnswer")} content={summary.aggregate.answer} unavailableMessage="explainability.messages.primerAnswerWasNotCapturedRunWithContentOrDebugMode" testId="drift-primer-answer" preview={false} /><CapturedContentViewer buttonLabel="explainability.actions.viewAggregatedFollowUps" title={t("explainability.labels.aggregatedFollowUpQueries")} content={summary.aggregate.followUpQueries?.join("\n") ?? null} unavailableMessage="explainability.messages.primerFollowUpsNotCaptured" testId="drift-primer-followups" preview={false} /></div></section>}
    </div>
  )
}

function PrimerFold({ fold }: { fold: DriftPrimerFoldView }): React.ReactElement {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  return <div className="min-w-0 rounded border bg-background/50 p-2"><button type="button" className="flex w-full min-w-0 items-center gap-2 text-left text-xs" aria-expanded={expanded} aria-label={t(expanded ? "explainability.actions.collapsePrimerFold" : "explainability.actions.expandPrimerFold", { fold: fold.foldIndex + 1 })} onClick={() => setExpanded((value) => !value)}>{expanded ? <ChevronDown className="size-3.5 shrink-0" /> : <ChevronRight className="size-3.5 shrink-0" />}<span className="min-w-0 flex-1 font-medium">{t("explainability.labels.foldValue", { value: fold.foldIndex + 1 })}</span><span className="shrink-0 text-muted-foreground">{t("graph.counts.countReport", { count: fold.reportIds.length })}{fold.score === undefined ? "" : ` · ${t("explainability.labels.scoreValue", { value: fold.score.toFixed(1) })}`}{fold.followUpCount === undefined ? "" : ` · ${t("explainability.counts.countFollowUp", { count: fold.followUpCount })}`}</span></button>{expanded ? <div className="mt-2 min-w-0 space-y-2 border-t pt-2"><p className="break-all font-mono text-[10px] text-muted-foreground">{fold.reportIds.length === 0 ? t("explainability.labels.0Reports") : fold.reportIds.join("\n")}</p><div className="flex flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewPrimerPrompt" title={t("explainability.labels.primerPromptTitle", { fold: fold.foldIndex + 1 })} content={fold.exactPrompt} unavailableMessage="explainability.messages.primerPromptWasNotCapturedRunWithContentOrDebugMode" testId={`drift-fold-${fold.foldIndex}-prompt`} />{fold.completed ? <CapturedContentViewer buttonLabel="explainability.actions.viewRawPrimerResponse" title={t("explainability.labels.primerRawResponseTitle", { fold: fold.foldIndex + 1 })} content={fold.rawResponse} unavailableMessage="explainability.messages.primerRawResponseNotCaptured" testId={`drift-fold-${fold.foldIndex}-response`} preview={false} /> : null}<CapturedContentViewer buttonLabel="explainability.actions.viewParsedAnswer" title={t("explainability.labels.primerParsedAnswerTitle", { fold: fold.foldIndex + 1 })} content={fold.intermediateAnswer} unavailableMessage="explainability.messages.primerParsedAnswerNotCaptured" testId={`drift-fold-${fold.foldIndex}-answer`} preview={false} /><CapturedContentViewer buttonLabel="explainability.actions.viewParsedFollowUps" title={t("explainability.labels.primerFollowUpsTitle", { fold: fold.foldIndex + 1 })} content={fold.followUpQueries?.join("\n") ?? null} unavailableMessage="explainability.messages.parsedFollowUpsNotCaptured" testId={`drift-fold-${fold.foldIndex}-followups`} preview={false} /></div><LlmMetrics view={fold} /></div> : null}</div>
}

function ExplorationContent({ summary }: { summary: Extract<DriftSemanticStep, { kind: "drift-exploration" }>["summary"] }): React.ReactElement {
  const { t } = useTranslation()
  const [showAllActions, setShowAllActions] = useState(false)
  const [showAllEdges, setShowAllEdges] = useState(false)
  const actions = showAllActions ? summary.nodes : summary.nodes.slice(0, INITIAL_ACTIONS)
  const edges = showAllEdges ? summary.edges : summary.edges.slice(0, INITIAL_EDGES)
  return <div className="mt-3 min-w-0 space-y-3"><section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.drift.depthDecisionsLabel")}><h4 className="text-xs font-semibold">{t("explainability.drift.depthDecisionsHeading")}</h4>{summary.started && summary.maxDepth === 0 ? <p className="mt-2 text-xs text-muted-foreground">{t("explainability.messages.noFollowUpDepthsConfigured")}</p> : null}<div className="mt-2 space-y-2">{summary.depths.map((depth) => <div key={depth.depthIndex} className="rounded border bg-background/50 p-2 text-xs"><div className="flex items-center justify-between gap-2"><span className="font-medium">{t("explainability.labels.depthValue", { value: depth.depthIndex + 1 })}</span><span className="text-muted-foreground">{t("explainability.labels.depthSelectionSummary", { incomplete: depth.candidateActionIds.length, selected: depth.selectedActionIds.length })}</span></div><p className="mt-1 text-[10px] text-muted-foreground">{t("explainability.labels.randomlySelectedFromIncompleteActions")}</p><div className="mt-2 grid gap-1 sm:grid-cols-2"><ActionIds label="explainability.labels.candidates" ids={depth.candidateActionIds} /><ActionIds label="explainability.actions.selected" ids={depth.selectedActionIds} /></div>{depth.selectedActionIds.length === 0 ? <p className="mt-2 text-xs text-muted-foreground">{t("explainability.messages.explorationStoppedNoIncompleteActionsSelected")}</p> : null}</div>)}</div></section><section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.labels.actionGraph")}><h4 className="text-xs font-semibold">{t("explainability.labels.actionGraph")}</h4><p className="mt-1 text-[10px] text-muted-foreground">{t("explainability.messages.actionGraphNotice")}</p><div className="mt-2 space-y-2">{actions.map((node) => <ActionNode key={node.actionId} node={node} />)}</div>{summary.nodes.length > INITIAL_ACTIONS ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllActions} aria-label={showAllActions ? t("explainability.actions.showFewerDriftActions") : t("explainability.counts.showAllCountDriftActions", { count: summary.nodes.length })} onClick={() => setShowAllActions((value) => !value)}>{showAllActions ? t("explainability.actions.showFewer") : t("explainability.counts.showAllCountActions", { count: summary.nodes.length })}</Button> : null}<div className="mt-3"><h5 className="text-xs font-medium">{t("explainability.counts.edgesCount", { count: summary.edges.length })}</h5><div className="mt-1 flex flex-wrap gap-1">{edges.map((edge) => <Badge key={edge.ordinal} variant="outline">#{edge.sourceActionId} → #{edge.targetActionId}</Badge>)}</div>{summary.edges.length > INITIAL_EDGES ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllEdges} aria-label={showAllEdges ? t("explainability.actions.showFewerDriftEdges") : t("explainability.counts.showAllCountDriftEdges", { count: summary.edges.length })} onClick={() => setShowAllEdges((value) => !value)}>{showAllEdges ? t("explainability.actions.showFewer") : t("explainability.counts.showAllCountEdges", { count: summary.edges.length })}</Button> : null}</div></section></div>
}

function ActionNode({ node }: { node: DriftActionNodeView }): React.ReactElement {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  return <div className="min-w-0 rounded border bg-background/50 p-2"><button type="button" className="flex w-full min-w-0 items-center gap-2 text-left text-xs" aria-expanded={expanded} aria-label={t(expanded ? "explainability.actions.collapseActionId" : "explainability.actions.expandActionId", { id: node.actionId })} onClick={() => setExpanded((value) => !value)}>{expanded ? <ChevronDown className="size-3.5 shrink-0" /> : <ChevronRight className="size-3.5 shrink-0" />}<span className="min-w-0 flex-1 font-medium">Action #{node.actionId}{node.status === "root" ? ` · ${t("explainability.labels.root")}` : ""}</span><Badge variant="outline">{actionStatus(t, node)}</Badge></button><p className="mt-1 min-w-0 truncate text-[10px] text-muted-foreground" title={node.query ?? t("explainability.labels.queryNotCaptured")}>{node.query ?? t("explainability.labels.queryNotCaptured")}</p>{node.outgoingEdges.map((edge) => <p key={edge.ordinal} className="ml-5 text-[10px] text-muted-foreground">→ #{edge.targetActionId}</p>)}{expanded ? <div className="mt-2 space-y-2 border-t pt-2">{node.attempts.length === 0 ? <p className="text-xs text-muted-foreground">{t("explainability.labels.notExplored")}</p> : node.attempts.map((attempt, index) => <ActionAttempt key={attempt.identity} attempt={attempt} index={index} />)}</div> : null}</div>
}

function ActionAttempt({ attempt, index }: { attempt: DriftActionAttemptView; index: number }): React.ReactElement {
  const { t } = useTranslation()
  const status = t(attempt.status === "incomplete" ? "explainability.labels.remainsIncomplete" : attempt.status === "completed_empty" ? "explainability.labels.completedEmptyAnswer" : attempt.status === "completed" ? "explainability.labels.completed" : "explainability.labels.inProgress")
  return <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={t("explainability.labels.actionAttempt", { action: attempt.actionId, attempt: index + 1 })}><div className="flex items-center justify-between gap-2 text-xs"><span className="font-medium">{t("explainability.labels.attemptDepth", { attempt: index + 1, depth: attempt.depthIndex + 1 })}</span><Badge variant="outline">{status}</Badge></div><dl className="mt-2 grid grid-cols-2 gap-2 text-xs"><Metric label="explainability.labels.score" value={attempt.score === undefined ? t("explainability.labels.noFiniteScore") : attempt.score.toFixed(1)} />{attempt.followUpCount === undefined ? null : <Metric label="explainability.labels.followUps" value={attempt.followUpCount} />}</dl><div className="mt-2 flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="explainability.actions.viewQuery" title={t("explainability.labels.actionIdQuery", { id: attempt.actionId })} content={attempt.query} unavailableMessage="explainability.messages.actionQueryWasNotCapturedRunWithContentOrDebugMode" testId={`drift-action-${attempt.actionId}-${index}-query`} preview={false} /><CapturedContentViewer buttonLabel="explainability.actions.viewLocalContext" title={t("explainability.labels.actionIdLocalContext", { id: attempt.actionId })} content={attempt.context} unavailableMessage="explainability.messages.actionContextNotCaptured" testId={`drift-action-${attempt.actionId}-${index}-context`} /><CapturedContentViewer buttonLabel="explainability.actions.viewActionPrompt" title={t("explainability.labels.actionIdPrompt", { id: attempt.actionId })} content={attempt.exactPrompt} unavailableMessage="explainability.messages.actionPromptWasNotCapturedRunWithContentOrDebugMode" testId={`drift-action-${attempt.actionId}-${index}-prompt`} />{attempt.rawResponse === null ? null : <CapturedContentViewer buttonLabel="explainability.actions.viewRawActionResponse" title={t("explainability.labels.actionIdRawResponse", { id: attempt.actionId })} content={attempt.rawResponse} unavailableMessage="explainability.messages.actionRawResponseNotCaptured" testId={`drift-action-${attempt.actionId}-${index}-response`} preview={false} />}<CapturedContentViewer buttonLabel="explainability.actions.viewParsedAnswer" title={t("explainability.labels.actionIdParsedAnswer", { id: attempt.actionId })} content={attempt.answer} unavailableMessage="explainability.messages.actionAnswerNotCaptured" testId={`drift-action-${attempt.actionId}-${index}-answer`} preview={false} /><CapturedContentViewer buttonLabel="explainability.actions.viewGeneratedFollowUps" title={t("explainability.labels.actionIdGeneratedFollowUps", { id: attempt.actionId })} content={attempt.followUpQueries?.join("\n") ?? null} unavailableMessage="explainability.messages.actionFollowUpsNotCaptured" testId={`drift-action-${attempt.actionId}-${index}-followups`} preview={false} /></div><LlmMetrics view={attempt} /></section>
}

function actionStatus(t: TFunction, node: DriftActionNodeView): string {
  if (node.status === "root") return t("explainability.labels.root")
  if (node.status === "not_explored") return t("explainability.labels.notExplored")
  if (node.status === "incomplete") return t("explainability.counts.remainsIncompleteCountAttempt", { count: node.attempts.length })
  if (node.status === "completed_empty") return t("explainability.counts.completedEmptyAnswerCountAttempt", { count: node.attempts.length })
  if (node.status === "completed") return t("explainability.counts.completedCountAttempt", { count: node.attempts.length })
  return t("explainability.labels.inProgress")
}

function ActionIds({ label, ids }: { label: StudioTranslationKey; ids: number[] }): React.ReactElement { const { t } = useTranslation(); return <div className="min-w-0"><span className="text-[10px] text-muted-foreground">{t(label)}</span><div className="mt-1 flex flex-wrap gap-1">{ids.length === 0 ? <span className="text-xs text-muted-foreground">{t("graph.labels.none")}</span> : ids.map((id) => <Badge key={id} variant="outline">#{id}</Badge>)}</div></div> }
function Metric({ label, value, breakAll = false }: { label: StudioTranslationKey; value: string | number; breakAll?: boolean }): React.ReactElement { const { t } = useTranslation(); return <div className="min-w-0 rounded border bg-background/50 px-2 py-1.5"><dt className="text-muted-foreground">{t(label)}</dt><dd className={`mt-0.5 font-medium ${breakAll ? "break-all" : "break-words"}`}>{value}</dd></div> }
function LlmMetrics({ view }: { view: { model?: string; inputTokens?: number; outputTokens?: number; elapsedMs?: number } }): React.ReactElement | null { const { t } = useTranslation(); const values = [view.model, view.inputTokens === undefined ? undefined : t("answer.counts.countInput", { count: view.inputTokens }), view.outputTokens === undefined ? undefined : t("answer.counts.countOutput", { count: view.outputTokens }), view.elapsedMs === undefined ? undefined : `${view.elapsedMs} ms`].filter((value): value is string => value !== undefined); return values.length === 0 ? null : <p className="mt-2 break-words text-[10px] text-muted-foreground">{values.join(" · ")}</p> }
