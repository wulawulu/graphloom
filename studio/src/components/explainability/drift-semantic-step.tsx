import { useState } from "react"
import { BrainCircuit, ChevronDown, ChevronRight, GitFork, Sparkles } from "lucide-react"

import type { ExplainabilityEnvelope } from "@/api/types"
import { CapturedContentViewer } from "@/components/explainability/captured-content-viewer"
import { TechnicalDetails } from "@/components/explainability/technical-details"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { DriftActionAttemptView, DriftActionNodeView, DriftPrimerFoldView, DriftSemanticStep } from "@/lib/semantic-drift"

interface DriftSemanticStepCardProps {
  step: DriftSemanticStep
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

const INITIAL_ACTIONS = 20
const INITIAL_EDGES = 30
const INITIAL_FOLDS = 12

export function DriftSemanticStepCard({ step, onFocusGraph }: DriftSemanticStepCardProps): React.ReactElement {
  return (
    <article className="min-w-0 rounded-md border bg-card/70 p-3" aria-label={step.title}>
      <div className="flex min-w-0 gap-2">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-primary"><StepIcon kind={step.kind} /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{step.title}</h3><StepSummary step={step} /></div>
      </div>
      <StepContent step={step} />
      <TechnicalDetails rawEvents={step.rawEvents} onFocusGraph={onFocusGraph} />
    </article>
  )
}

function StepIcon({ kind }: { kind: DriftSemanticStep["kind"] }): React.ReactElement {
  if (kind === "drift-primer-ranking") return <BrainCircuit className="size-4" />
  if (kind === "drift-exploration") return <GitFork className="size-4" />
  return <Sparkles className="size-4" />
}

function StepSummary({ step }: { step: DriftSemanticStep }): React.ReactElement {
  if (step.kind === "drift-primer-ranking") {
    const summary = step.summary
    if (summary.aggregate === null) {
      const progress = summary.status === "hyde" ? "Generating HyDE expansion" : summary.status === "embedding" ? "Embedding expanded query" : summary.status === "ranking" ? "Ranking community reports" : "Running primer folds"
      return <p className="mt-1 text-xs text-muted-foreground">{progress}</p>
    }
    return <p className="mt-1 text-xs text-muted-foreground">{summary.rankedReports.length} reports ranked · {summary.folds.length} primer folds · {summary.aggregate.followUpCount} follow-up queries · Primer score {summary.aggregate.score.toFixed(1)}</p>
  }
  if (step.kind === "drift-exploration") {
    const summary = step.summary
    if (summary.activeDepth !== undefined && summary.nodeCount === undefined) {
      const selected = summary.depths.at(-1)?.selectedActionIds.length ?? 0
      return <p className="mt-1 text-xs text-muted-foreground">Depth {summary.activeDepth + 1} · {selected} actions selected</p>
    }
    return <p className="mt-1 text-xs text-muted-foreground">{summary.depths.length} depths · {summary.attempts.length} action attempts · {summary.nodeCount ?? summary.nodes.length} action nodes · {summary.edgeCount ?? summary.edges.length} edges</p>
  }
  const summary = step.summary
  if (!summary.built) return <p className="mt-1 text-xs text-muted-foreground">Waiting for final synthesis</p>
  if (!summary.generated) return <p className="mt-1 text-xs text-muted-foreground">Generating final synthesis</p>
  return <p className="mt-1 text-xs text-muted-foreground">{summary.includedAnswerCount ?? 0} answers included · {summary.nodeCount ?? 0} action nodes · {summary.edgeCount ?? 0} exploration edges</p>
}

function StepContent({ step }: { step: DriftSemanticStep }): React.ReactNode {
  if (step.kind === "drift-primer-ranking") return <PrimerContent summary={step.summary} />
  if (step.kind === "drift-exploration") return <ExplorationContent summary={step.summary} />
  const summary = step.summary
  return (
    <div className="mt-3 min-w-0 space-y-3">
      <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label="Reduce selection">
        <h4 className="text-xs font-semibold">Included in Reduce</h4>
        {summary.includedActionIds.length === 0 ? <p className="mt-1 text-xs text-muted-foreground">No included answers recorded yet.</p> : <div className="mt-2 flex flex-wrap gap-1">{summary.includedActionIds.map((id, index) => <Badge key={`${id}:${index}`} variant="outline">#{id}</Badge>)}</div>}
      </section>
      <div className="flex min-w-0 flex-wrap gap-2">
        <CapturedContentViewer buttonLabel="View DRIFT State" title="DRIFT State" content={summary.exactStateContext} unavailableMessage="DRIFT state content was not captured. Run with Content or Debug mode to inspect exact state.to_json() output." testId="exact-drift-state" exactTabLabel="Exact source" copyLabel="Copy exact DRIFT state" description="Exact source is the backend state.to_json() output; Preview is presentation only." />
        <CapturedContentViewer buttonLabel="View Reduce Context" title="Reduce Context" content={summary.exactReduceContext} unavailableMessage="Reduce context was not captured. Run with Content or Debug mode to inspect exact python_list_repr() output." testId="exact-drift-reduce-context" exactTabLabel="Exact source" copyLabel="Copy exact Reduce context" description="Exact source is the backend Python-list representation and is never reconstructed in Studio." />
        <CapturedContentViewer buttonLabel="View Reduce Prompt" title="Reduce Prompt" content={summary.exactPrompt} unavailableMessage="Reduce prompt was not captured. Run with Content or Debug mode." testId="exact-drift-reduce-prompt" />
        {summary.generated ? <CapturedContentViewer buttonLabel="View Raw Reduce Response" title="Raw Reduce Response" content={summary.rawResponse} unavailableMessage="Raw Reduce response was not captured. Run with Content or Debug mode." testId="raw-drift-reduce-response" preview={false} /> : null}
      </div>
      <LlmMetrics view={summary} />
    </div>
  )
}

function PrimerContent({ summary }: { summary: Extract<DriftSemanticStep, { kind: "drift-primer-ranking" }>["summary"] }): React.ReactElement {
  const [showAllFolds, setShowAllFolds] = useState(false)
  const [showAllReports, setShowAllReports] = useState(false)
  const reports = showAllReports ? summary.rankedReports : summary.rankedReports.slice(0, 20)
  const folds = showAllFolds ? summary.folds : summary.folds.slice(0, INITIAL_FOLDS)
  return (
    <div className="mt-3 min-w-0 space-y-3">
      {summary.hyde === null ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label="HyDE Expansion"><h4 className="text-xs font-semibold">HyDE Expansion</h4><dl className="mt-2 grid grid-cols-2 gap-2 text-xs"><Metric label="Template" value={`Report ${summary.hyde.templateShortId}`} /><Metric label="Stable ID" value={summary.hyde.templateReportId} breakAll /><Metric label="Community" value={summary.hyde.templateCommunityId} breakAll /><Metric label="Selection" value={`${summary.hyde.templateIndex + 1} / ${summary.hyde.reportCount}`} /></dl>{summary.hyde.completed ? <p className="mt-2 text-xs text-muted-foreground">{summary.hyde.usedOriginalQuery ? "Empty expansion · original query used" : "HyDE expansion used"}</p> : <p className="mt-2 text-xs text-muted-foreground">Generating expansion...</p>}<div className="mt-2 flex flex-wrap gap-2"><CapturedContentViewer buttonLabel="View HyDE Prompt" title="HyDE Prompt" content={summary.hyde.exactPrompt} unavailableMessage="HyDE prompt was not captured. Run with Content or Debug mode." testId="exact-drift-hyde-prompt" />{summary.hyde.completed ? <CapturedContentViewer buttonLabel="View Raw HyDE Response" title="Raw HyDE Response" content={summary.hyde.rawResponse} unavailableMessage="Raw HyDE response was not captured. Run with Content or Debug mode." testId="raw-drift-hyde-response" preview={false} /> : null}</div>{summary.hyde.completed ? <div className="mt-2"><CapturedContentViewer buttonLabel="View Effective Query" title="Effective Embedding Query" content={summary.hyde.effectiveQuery} unavailableMessage="Effective query content was not captured. Run with Content or Debug mode." testId="exact-drift-effective-query" preview={false} /></div> : null}</section>}
      {summary.rankedReports.length === 0 ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label="Ranked Reports"><h4 className="text-xs font-semibold">Ranked Reports</h4><div className="mt-2 divide-y overflow-hidden rounded border bg-background/50">{reports.map((report) => <div key={`${report.rank}:${report.report_id}`} className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 px-2 py-2 text-xs"><span className="font-mono">#{report.rank}</span><div className="min-w-0"><p className="truncate font-medium" title={report.short_id}>Report {report.short_id}</p><p className="break-all font-mono text-[10px] text-muted-foreground">{report.report_id}</p><p className="break-all text-[10px] text-muted-foreground">Community {report.community_id}</p></div><span className="font-mono">{report.similarity.toFixed(4)}</span></div>)}</div>{summary.rankedReports.length > 20 ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllReports} aria-label={showAllReports ? "Show fewer ranked reports" : `Show all ${summary.rankedReports.length} ranked reports`} onClick={() => setShowAllReports((value) => !value)}>{showAllReports ? "Show fewer" : `Show all ${summary.rankedReports.length}`}</Button> : null}</section>}
      {summary.folds.length === 0 ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label="Primer Folds"><h4 className="text-xs font-semibold">Primer Folds</h4><div className="mt-2 space-y-2">{folds.map((fold) => <PrimerFold key={fold.foldIndex} fold={fold} />)}</div>{summary.folds.length > INITIAL_FOLDS ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllFolds} aria-label={showAllFolds ? "Show fewer Primer folds" : `Show all ${summary.folds.length} Primer folds`} onClick={() => setShowAllFolds((value) => !value)}>{showAllFolds ? "Show fewer" : `Show all ${summary.folds.length}`}</Button> : null}</section>}
      {summary.aggregate === null ? null : <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label="Primer Aggregate"><h4 className="text-xs font-semibold">Primer Aggregate</h4><dl className="mt-2 grid grid-cols-2 gap-2 text-xs"><Metric label="Score" value={summary.aggregate.score.toFixed(1)} /><Metric label="Follow-ups" value={summary.aggregate.followUpCount} /><Metric label="Root action" value={`#${summary.aggregate.rootActionId}`} /><Metric label="Target IDs" value={summary.aggregate.followUpActionIds.map((id) => `#${id}`).join(" ") || "None"} /></dl><div className="mt-2 flex flex-wrap gap-2"><CapturedContentViewer buttonLabel="View Primer Answer" title="Primer Intermediate Answer" content={summary.aggregate.answer} unavailableMessage="Primer answer was not captured. Run with Content or Debug mode." testId="drift-primer-answer" preview={false} /><CapturedContentViewer buttonLabel="View Aggregated Follow-ups" title="Aggregated Follow-up Queries" content={summary.aggregate.followUpQueries?.join("\n") ?? null} unavailableMessage="Primer follow-up text was not captured. Run with Content or Debug mode." testId="drift-primer-followups" preview={false} /></div></section>}
    </div>
  )
}

function PrimerFold({ fold }: { fold: DriftPrimerFoldView }): React.ReactElement {
  const [expanded, setExpanded] = useState(false)
  return <div className="min-w-0 rounded border bg-background/50 p-2"><button type="button" className="flex w-full min-w-0 items-center gap-2 text-left text-xs" aria-expanded={expanded} aria-label={`${expanded ? "Collapse" : "Expand"} Primer Fold ${fold.foldIndex + 1}`} onClick={() => setExpanded((value) => !value)}>{expanded ? <ChevronDown className="size-3.5 shrink-0" /> : <ChevronRight className="size-3.5 shrink-0" />}<span className="min-w-0 flex-1 font-medium">Fold {fold.foldIndex + 1}</span><span className="shrink-0 text-muted-foreground">{fold.reportIds.length} reports{fold.score === undefined ? "" : ` · Score ${fold.score.toFixed(1)}`}{fold.followUpCount === undefined ? "" : ` · ${fold.followUpCount} follow-ups`}</span></button>{expanded ? <div className="mt-2 min-w-0 space-y-2 border-t pt-2"><p className="break-all font-mono text-[10px] text-muted-foreground">{fold.reportIds.length === 0 ? "0 reports" : fold.reportIds.join("\n")}</p><div className="flex flex-wrap gap-2"><CapturedContentViewer buttonLabel="View Primer Prompt" title={`Primer Fold ${fold.foldIndex + 1} Prompt`} content={fold.exactPrompt} unavailableMessage="Primer prompt was not captured. Run with Content or Debug mode." testId={`drift-fold-${fold.foldIndex}-prompt`} />{fold.completed ? <CapturedContentViewer buttonLabel="View Raw Primer Response" title={`Primer Fold ${fold.foldIndex + 1} Raw Response`} content={fold.rawResponse} unavailableMessage="Raw Primer response was not captured. Run with Content or Debug mode." testId={`drift-fold-${fold.foldIndex}-response`} preview={false} /> : null}<CapturedContentViewer buttonLabel="View Parsed Answer" title={`Primer Fold ${fold.foldIndex + 1} Parsed Answer`} content={fold.intermediateAnswer} unavailableMessage="Parsed Primer answer was not captured. Run with Content or Debug mode." testId={`drift-fold-${fold.foldIndex}-answer`} preview={false} /><CapturedContentViewer buttonLabel="View Parsed Follow-ups" title={`Primer Fold ${fold.foldIndex + 1} Follow-ups`} content={fold.followUpQueries?.join("\n") ?? null} unavailableMessage="Parsed follow-up text was not captured. Run with Content or Debug mode." testId={`drift-fold-${fold.foldIndex}-followups`} preview={false} /></div><LlmMetrics view={fold} /></div> : null}</div>
}

function ExplorationContent({ summary }: { summary: Extract<DriftSemanticStep, { kind: "drift-exploration" }>["summary"] }): React.ReactElement {
  const [showAllActions, setShowAllActions] = useState(false)
  const [showAllEdges, setShowAllEdges] = useState(false)
  const actions = showAllActions ? summary.nodes : summary.nodes.slice(0, INITIAL_ACTIONS)
  const edges = showAllEdges ? summary.edges : summary.edges.slice(0, INITIAL_EDGES)
  return <div className="mt-3 min-w-0 space-y-3"><section className="min-w-0 rounded border bg-muted/20 p-2" aria-label="Depth decisions"><h4 className="text-xs font-semibold">Depth Decisions</h4>{summary.started && summary.maxDepth === 0 ? <p className="mt-2 text-xs text-muted-foreground">No follow-up depths configured.</p> : null}<div className="mt-2 space-y-2">{summary.depths.map((depth) => <div key={depth.depthIndex} className="rounded border bg-background/50 p-2 text-xs"><div className="flex items-center justify-between gap-2"><span className="font-medium">Depth {depth.depthIndex + 1}</span><span className="text-muted-foreground">{depth.candidateActionIds.length} incomplete · {depth.selectedActionIds.length} randomly selected</span></div><p className="mt-1 text-[10px] text-muted-foreground">Randomly selected from incomplete actions</p><div className="mt-2 grid gap-1 sm:grid-cols-2"><ActionIds label="Candidates" ids={depth.candidateActionIds} /><ActionIds label="Selected" ids={depth.selectedActionIds} /></div>{depth.selectedActionIds.length === 0 ? <p className="mt-2 text-xs text-muted-foreground">Exploration stopped — no incomplete actions selected.</p> : null}</div>)}</div></section><section className="min-w-0 rounded border bg-muted/20 p-2" aria-label="Action Graph"><h4 className="text-xs font-semibold">Action Graph</h4><p className="mt-1 text-[10px] text-muted-foreground">Follow-up reasoning graph. Nodes are exact query identities; duplicate edges and multiple parents are preserved.</p><div className="mt-2 space-y-2">{actions.map((node) => <ActionNode key={node.actionId} node={node} />)}</div>{summary.nodes.length > INITIAL_ACTIONS ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllActions} aria-label={showAllActions ? "Show fewer DRIFT actions" : `Show all ${summary.nodes.length} DRIFT actions`} onClick={() => setShowAllActions((value) => !value)}>{showAllActions ? "Show fewer" : `Show all ${summary.nodes.length} actions`}</Button> : null}<div className="mt-3"><h5 className="text-xs font-medium">Edges · {summary.edges.length}</h5><div className="mt-1 flex flex-wrap gap-1">{edges.map((edge) => <Badge key={edge.ordinal} variant="outline">#{edge.sourceActionId} → #{edge.targetActionId}</Badge>)}</div>{summary.edges.length > INITIAL_EDGES ? <Button className="mt-2" variant="ghost" size="sm" aria-expanded={showAllEdges} aria-label={showAllEdges ? "Show fewer DRIFT edges" : `Show all ${summary.edges.length} DRIFT edges`} onClick={() => setShowAllEdges((value) => !value)}>{showAllEdges ? "Show fewer" : `Show all ${summary.edges.length} edges`}</Button> : null}</div></section></div>
}

function ActionNode({ node }: { node: DriftActionNodeView }): React.ReactElement {
  const [expanded, setExpanded] = useState(false)
  return <div className="min-w-0 rounded border bg-background/50 p-2"><button type="button" className="flex w-full min-w-0 items-center gap-2 text-left text-xs" aria-expanded={expanded} aria-label={`${expanded ? "Collapse" : "Expand"} Action ${node.actionId}`} onClick={() => setExpanded((value) => !value)}>{expanded ? <ChevronDown className="size-3.5 shrink-0" /> : <ChevronRight className="size-3.5 shrink-0" />}<span className="min-w-0 flex-1 font-medium">Action #{node.actionId}{node.status === "root" ? " · Root" : ""}</span><Badge variant="outline">{actionStatus(node)}</Badge></button><p className="mt-1 min-w-0 truncate text-[10px] text-muted-foreground" title={node.query ?? "Query not captured"}>{node.query ?? "Query not captured"}</p>{node.outgoingEdges.map((edge) => <p key={edge.ordinal} className="ml-5 text-[10px] text-muted-foreground">→ #{edge.targetActionId}</p>)}{expanded ? <div className="mt-2 space-y-2 border-t pt-2">{node.attempts.length === 0 ? <p className="text-xs text-muted-foreground">Not explored</p> : node.attempts.map((attempt, index) => <ActionAttempt key={attempt.identity} attempt={attempt} index={index} />)}</div> : null}</div>
}

function ActionAttempt({ attempt, index }: { attempt: DriftActionAttemptView; index: number }): React.ReactElement {
  const status = attempt.status === "incomplete" ? "Remains incomplete" : attempt.status === "completed_empty" ? "Completed · empty answer" : attempt.status === "completed" ? "Completed" : "In progress"
  return <section className="min-w-0 rounded border bg-muted/20 p-2" aria-label={`Action ${attempt.actionId} attempt ${index + 1}`}><div className="flex items-center justify-between gap-2 text-xs"><span className="font-medium">Attempt {index + 1} · Depth {attempt.depthIndex + 1}</span><Badge variant="outline">{status}</Badge></div><dl className="mt-2 grid grid-cols-2 gap-2 text-xs"><Metric label="Score" value={attempt.score === undefined ? "No finite score" : attempt.score.toFixed(1)} />{attempt.followUpCount === undefined ? null : <Metric label="Follow-ups" value={attempt.followUpCount} />}</dl><div className="mt-2 flex min-w-0 flex-wrap gap-2"><CapturedContentViewer buttonLabel="View Query" title={`Action #${attempt.actionId} Query`} content={attempt.query} unavailableMessage="Action query was not captured. Run with Content or Debug mode." testId={`drift-action-${attempt.actionId}-${index}-query`} preview={false} /><CapturedContentViewer buttonLabel="View Local Context" title={`Action #${attempt.actionId} Local Context`} content={attempt.context} unavailableMessage="Action Local context was not captured. Run with Content or Debug mode." testId={`drift-action-${attempt.actionId}-${index}-context`} /><CapturedContentViewer buttonLabel="View Action Prompt" title={`Action #${attempt.actionId} Prompt`} content={attempt.exactPrompt} unavailableMessage="Action prompt was not captured. Run with Content or Debug mode." testId={`drift-action-${attempt.actionId}-${index}-prompt`} />{attempt.rawResponse === null ? null : <CapturedContentViewer buttonLabel="View Raw Action Response" title={`Action #${attempt.actionId} Raw Response`} content={attempt.rawResponse} unavailableMessage="Raw Action response was not captured. Run with Content or Debug mode." testId={`drift-action-${attempt.actionId}-${index}-response`} preview={false} />}<CapturedContentViewer buttonLabel="View Parsed Answer" title={`Action #${attempt.actionId} Parsed Answer`} content={attempt.answer} unavailableMessage="Parsed Action answer was not captured, or no completed answer exists." testId={`drift-action-${attempt.actionId}-${index}-answer`} preview={false} /><CapturedContentViewer buttonLabel="View Generated Follow-ups" title={`Action #${attempt.actionId} Generated Follow-ups`} content={attempt.followUpQueries?.join("\n") ?? null} unavailableMessage="Generated follow-up text was not captured. Run with Content or Debug mode." testId={`drift-action-${attempt.actionId}-${index}-followups`} preview={false} /></div><LlmMetrics view={attempt} /></section>
}

function actionStatus(node: DriftActionNodeView): string {
  if (node.status === "root") return "Root"
  if (node.status === "not_explored") return "Not explored"
  if (node.status === "incomplete") return `Remains incomplete · ${node.attempts.length} attempts`
  if (node.status === "completed_empty") return `Completed · empty answer · ${node.attempts.length} attempts`
  if (node.status === "completed") return `Completed · ${node.attempts.length} attempts`
  return "In progress"
}

function ActionIds({ label, ids }: { label: string; ids: number[] }): React.ReactElement { return <div className="min-w-0"><span className="text-[10px] text-muted-foreground">{label}</span><div className="mt-1 flex flex-wrap gap-1">{ids.length === 0 ? <span className="text-xs text-muted-foreground">None</span> : ids.map((id) => <Badge key={id} variant="outline">#{id}</Badge>)}</div></div> }
function Metric({ label, value, breakAll = false }: { label: string; value: string | number; breakAll?: boolean }): React.ReactElement { return <div className="min-w-0 rounded border bg-background/50 px-2 py-1.5"><dt className="text-muted-foreground">{label}</dt><dd className={`mt-0.5 font-medium ${breakAll ? "break-all" : "break-words"}`}>{value}</dd></div> }
function LlmMetrics({ view }: { view: { model?: string; inputTokens?: number; outputTokens?: number; elapsedMs?: number } }): React.ReactElement | null { const values = [view.model, view.inputTokens === undefined ? undefined : `${view.inputTokens} input`, view.outputTokens === undefined ? undefined : `${view.outputTokens} output`, view.elapsedMs === undefined ? undefined : `${view.elapsedMs} ms`].filter((value): value is string => value !== undefined); return values.length === 0 ? null : <p className="mt-2 break-words text-[10px] text-muted-foreground">{values.join(" · ")}</p> }
