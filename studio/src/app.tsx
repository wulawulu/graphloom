import { lazy, Suspense, useCallback, useMemo, useState } from "react"

import type { ExplainabilityEnvelope, GraphSubgraphRequest, StartQueryResponse } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"
import { StudioShell } from "@/components/layout/studio-shell"
import { QueryComposer } from "@/components/query/query-composer"
import { RunList } from "@/components/runs/run-list"
import { Skeleton } from "@/components/ui/skeleton"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useExplainabilityStream } from "@/hooks/use-explainability-stream"
import { useRun } from "@/hooks/use-run"
import { useRunHistory } from "@/hooks/use-run-history"
import { highlightFromEvent } from "@/lib/explainability"
import { QueryWorkspace } from "@/components/workspace/query-workspace"

const GraphExplorer = lazy(() => import("@/components/graph/graph-explorer").then((module) => ({ default: module.GraphExplorer })))
const AnswerPanel = lazy(() => import("@/components/result/answer-panel").then((module) => ({ default: module.AnswerPanel })))

export function App(): React.ReactElement {
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  const [graphFocus, setGraphFocus] = useState<(GraphSubgraphRequest & { revision: number }) | null>(null)
  const [mobileTab, setMobileTab] = useState("query")
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
    setSelectedRunId(runId)
    setGraphFocus(null)
  }, [])

  const clearGraphFocus = useCallback(() => setGraphFocus(null), [])

  const onAccepted = useCallback((response: StartQueryResponse) => {
    selectRun(response.run_id)
    setMobileTab("query")
    refreshHistory()
  }, [refreshHistory, selectRun])

  const onFocusGraph = useCallback((envelope: ExplainabilityEnvelope) => {
    const highlight = highlightFromEvent(envelope.record.event)
    if (highlight === null) return
    setGraphFocus((current) => ({
      entity_ids: highlight.entityIds,
      relationship_ids: highlight.relationshipIds,
      depth: 1,
      max_entities: 80,
      max_relationships: 160,
      revision: (current?.revision ?? 0) + 1,
    }))
    setMobileTab("graph")
  }, [])

  const queryWorkspace = useMemo(() => (
    <QueryWorkspace
      composer={<QueryComposer onAccepted={onAccepted} />}
      answer={<Suspense fallback={<PanelLoading label="Loading Final Answer" />}><AnswerPanel runId={selectedRunId} result={selected.result} loading={selected.loading} /></Suspense>}
      trace={<Timeline runId={selectedRunId} envelopes={stream.envelopes} streamStatus={stream.status} onFocusGraph={onFocusGraph} />}
      runs={<RunList runs={history.runs} selectedRunId={selectedRunId} loading={history.loading} error={history.error} hasMore={history.cursor !== null} onSelect={selectRun} onRefresh={history.refresh} onLoadMore={history.loadMore} />}
    />
  ), [history.cursor, history.error, history.loadMore, history.loading, history.refresh, history.runs, onAccepted, onFocusGraph, selectRun, selected.loading, selected.result, selectedRunId, stream.envelopes, stream.status])

  return (
    <TooltipProvider delayDuration={250}>
      <StudioShell
        queryWorkspace={queryWorkspace}
        graph={<Suspense fallback={<PanelLoading label="Loading Graph Explorer" />}><GraphExplorer runId={selectedRunId} focusIntent={graphFocus} onClearFocus={clearGraphFocus} /></Suspense>}
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
