import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react"

import type { ExplainabilityEnvelope, StartQueryResponse } from "@/api/types"
import { StudioShell } from "@/components/layout/studio-shell"
import { QueryComposer } from "@/components/query/query-composer"
import { Skeleton } from "@/components/ui/skeleton"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useExplainabilityStream } from "@/hooks/use-explainability-stream"
import { useRun } from "@/hooks/use-run"
import { useRunHistory } from "@/hooks/use-run-history"
import { QaWorkspace } from "@/components/workspace/qa-workspace"
import { deriveFinalGraphFocus, highlightFromEvent } from "@/lib/explainability"
import type { GraphEmphasis, GraphEmphasisIntent } from "@/lib/citations"
import type { GraphFocusIntent, GraphInspectIntent } from "@/components/graph/graph-explorer"
import type { ExplainabilityRecordView } from "@/lib/semantic-timeline"

const GraphExplorer = lazy(() => import("@/components/graph/graph-explorer").then((module) => ({ default: module.GraphExplorer })))
const AnswerPanel = lazy(() => import("@/components/result/answer-panel").then((module) => ({ default: module.AnswerPanel })))

export function App(): React.ReactElement {
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  const [graphFocus, setGraphFocus] = useState<GraphFocusIntent | null>(null)
  const [citationEmphasis, setCitationEmphasis] = useState<GraphEmphasisIntent | null>(null)
  const [graphInspection, setGraphInspection] = useState<GraphInspectIntent | null>(null)
  const [mobileTab, setMobileTab] = useState("query")
  const [submittedQuestion, setSubmittedQuestion] = useState<string | null>(null)
  const [activeSubmittedRunId, setActiveSubmittedRunId] = useState<string | null>(null)
  const [showCurrentQa, setShowCurrentQa] = useState(false)
  const [composerRevision, setComposerRevision] = useState(0)
  const [graphNavigationRevision, setGraphNavigationRevision] = useState(0)
  const autoFocusedRunIds = useRef(new Set<string>())
  const history = useRunHistory()
  const selected = useRun(selectedRunId)
  const refreshHistory = history.refresh
  const refreshSelectedRun = selected.refresh

  const onTerminal = useCallback(() => {
    refreshSelectedRun()
    refreshHistory()
  }, [refreshHistory, refreshSelectedRun])
  const stream = useExplainabilityStream(selectedRunId, selected.run?.status, onTerminal)

  const selectRun = useCallback((runId: string | null) => {
    setActiveSubmittedRunId(null)
    setSubmittedQuestion(null)
    setShowCurrentQa(runId !== null)
    setSelectedRunId(runId)
    setGraphFocus(null)
    setCitationEmphasis(null)
    setGraphInspection(null)
  }, [])

  const clearGraphFocus = useCallback(() => setGraphFocus(null), [])
  const clearCitationEmphasis = useCallback(() => setCitationEmphasis(null), [])

  const onCitationEmphasis = useCallback((emphasis: GraphEmphasis) => {
    setCitationEmphasis((current) => ({ ...emphasis, revision: (current?.revision ?? 0) + 1 }))
    setMobileTab("graph")
  }, [])

  const onInspectCandidate = useCallback((candidate: ExplainabilityRecordView) => {
    if ((candidate.recordType !== "entity" && candidate.recordType !== "relationship") || candidate.stableId.length === 0) return
    setGraphInspection((current) => ({ candidate, revision: (current?.revision ?? 0) + 1 }))
    setMobileTab("graph")
  }, [])

  const onAccepted = useCallback((response: StartQueryResponse, query: string) => {
    setActiveSubmittedRunId(response.run_id)
    setSubmittedQuestion(query)
    setShowCurrentQa(true)
    setSelectedRunId(response.run_id)
    setGraphFocus(null)
    setCitationEmphasis(null)
    setGraphInspection(null)
    setMobileTab("query")
    refreshHistory()
  }, [refreshHistory])

  const onNewQuery = useCallback(() => {
    setActiveSubmittedRunId(null)
    setSubmittedQuestion(null)
    setShowCurrentQa(false)
    setGraphFocus(null)
    setCitationEmphasis(null)
    setGraphInspection(null)
    setGraphNavigationRevision((value) => value + 1)
    setComposerRevision((value) => value + 1)
    setMobileTab("query")
  }, [])

  const onFocusGraph = useCallback((envelope: ExplainabilityEnvelope) => {
    const highlight = highlightFromEvent(envelope.record.event)
    if (highlight === null) return
    setGraphFocus((current) => ({
      entity_ids: highlight.entityIds,
      relationship_ids: highlight.relationshipIds,
      depth: 1,
      max_entities: 80,
      max_relationships: 160,
      focusKind: "focus-target",
      revision: (current?.revision ?? 0) + 1,
    }))
    setMobileTab("graph")
  }, [])

  useEffect(() => {
    const runId = activeSubmittedRunId
    if (runId === null || runId !== selectedRunId || selected.run?.run_id !== runId || selected.run.status !== "completed") return
    const runEnvelopes = stream.envelopes.filter((envelope) => envelope.record.run_id === runId)
    if (!runEnvelopes.some((envelope) => envelope.record.event.type === "run_completed")) return
    if (autoFocusedRunIds.current.has(runId)) return
    const highlight = deriveFinalGraphFocus(runEnvelopes)
    if (highlight === null) return
    autoFocusedRunIds.current.add(runId)
    setGraphFocus((current) => ({
      entity_ids: highlight.entityIds,
      relationship_ids: highlight.relationshipIds,
      depth: 1,
      max_entities: 80,
      max_relationships: 160,
      focusKind: "final-context",
      revision: (current?.revision ?? 0) + 1,
    }))
    setMobileTab("graph")
  }, [activeSubmittedRunId, selected.run?.run_id, selected.run?.status, selectedRunId, stream.envelopes])

  const displayedRunId = showCurrentQa ? selectedRunId : null
  const displayedRun = selected.run?.run_id === displayedRunId ? selected.run : null
  const displayedResult = displayedRun === null ? { state: "waiting" as const } : selected.result
  const displayedLoading = displayedRunId !== null && (selected.loading || displayedRun === null)
  const displayedEnvelopes = displayedRunId === null
    ? []
    : stream.envelopes.filter((envelope) => envelope.record.run_id === displayedRunId)
  const displayedQuestion = displayedRunId === null
    ? null
    : activeSubmittedRunId === displayedRunId && submittedQuestion !== null
      ? submittedQuestion
      : displayedRun?.query ?? "Query hidden (metadata mode)"

  const queryWorkspace = (
    <QaWorkspace
      runId={displayedRunId}
      runStatus={displayedRun?.status}
      question={displayedQuestion}
      composer={<QueryComposer onAccepted={onAccepted} resetRevision={composerRevision} />}
      answer={<Suspense fallback={<PanelLoading label="Loading Final Answer" />}><AnswerPanel runId={displayedRunId} result={displayedResult} loading={displayedLoading} envelopes={displayedEnvelopes} onCitationEmphasis={onCitationEmphasis} /></Suspense>}
      envelopes={displayedEnvelopes}
      streamStatus={stream.status}
      runs={history.runs}
      historyLoading={history.loading}
      historyError={history.error}
      historyHasMore={history.cursor !== null}
      onFocusGraph={onFocusGraph}
      onInspectCandidate={onInspectCandidate}
      onNewQuery={onNewQuery}
      onSelectRun={selectRun}
      onRefreshHistory={history.refresh}
      onLoadMoreHistory={history.loadMore}
    />
  )

  return (
    <TooltipProvider delayDuration={250}>
      <StudioShell
        queryWorkspace={queryWorkspace}
        graph={<Suspense fallback={<PanelLoading label="Loading Graph Explorer" />}><GraphExplorer runId={selectedRunId} focusIntent={graphFocus} inspectIntent={graphInspection} navigationResetRevision={graphNavigationRevision} onClearFocus={clearGraphFocus} emphasisIntent={citationEmphasis} onClearEmphasis={clearCitationEmphasis} /></Suspense>}
        mobileTab={mobileTab}
        onMobileTabChange={setMobileTab}
      />
      <Toaster />
    </TooltipProvider>
  )
}

function PanelLoading({ label }: { label: string }): React.ReactElement {
  return <div className="space-y-3 p-4" aria-label={label}><Skeleton className="h-10" /><Skeleton className="h-40" /></div>
}
