import { lazy, Suspense, useCallback, useMemo, useState } from "react"

import type { ExplainabilityEnvelope, StartQueryResponse } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"
import { StudioShell } from "@/components/layout/studio-shell"
import { QueryComposer } from "@/components/query/query-composer"
import { RunList } from "@/components/runs/run-list"
import { Separator } from "@/components/ui/separator"
import { Skeleton } from "@/components/ui/skeleton"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useExplainabilityStream } from "@/hooks/use-explainability-stream"
import { useRun } from "@/hooks/use-run"
import { useRunHistory } from "@/hooks/use-run-history"
import { highlightFromEvent } from "@/lib/explainability"

const GraphExplorer = lazy(() => import("@/components/graph/graph-explorer").then((module) => ({ default: module.GraphExplorer })))
const AnswerPanel = lazy(() => import("@/components/result/answer-panel").then((module) => ({ default: module.AnswerPanel })))

export function App(): React.ReactElement {
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  const [entityHighlights, setEntityHighlights] = useState<ReadonlySet<string>>(new Set())
  const [relationshipHighlights, setRelationshipHighlights] = useState<ReadonlySet<string>>(new Set())
  const [focusRevision, setFocusRevision] = useState(0)
  const [mobileTab, setMobileTab] = useState("runs")
  const history = useRunHistory()
  const selected = useRun(selectedRunId)
  const refreshHistory = history.refresh
  const refreshSelectedRun = selected.refresh

  const onTerminal = useCallback(() => {
    refreshSelectedRun()
    refreshHistory()
  }, [refreshHistory, refreshSelectedRun])
  const stream = useExplainabilityStream(selectedRunId, selected.run?.status, onTerminal)

  const onAccepted = useCallback((response: StartQueryResponse) => {
    setSelectedRunId(response.run_id)
    setEntityHighlights(new Set())
    setRelationshipHighlights(new Set())
    setMobileTab("timeline")
    refreshHistory()
  }, [refreshHistory])

  const onFocusGraph = useCallback((envelope: ExplainabilityEnvelope) => {
    const highlight = highlightFromEvent(envelope.record.event)
    if (highlight === null) return
    setEntityHighlights(new Set(highlight.entityIds))
    setRelationshipHighlights(new Set(highlight.relationshipIds))
    setFocusRevision((value) => value + 1)
    setMobileTab("graph")
  }, [])

  const navigation = useMemo(() => (
    <div className="flex size-full min-h-0 flex-col">
      <QueryComposer onAccepted={onAccepted} />
      <Separator className="my-3" />
      <RunList runs={history.runs} selectedRunId={selectedRunId} loading={history.loading} error={history.error} hasMore={history.cursor !== null} onSelect={setSelectedRunId} onRefresh={history.refresh} onLoadMore={history.loadMore} />
    </div>
  ), [history.cursor, history.error, history.loadMore, history.loading, history.refresh, history.runs, onAccepted, selectedRunId])

  return (
    <TooltipProvider delayDuration={250}>
      <StudioShell
        navigation={navigation}
        timeline={<Timeline runId={selectedRunId} envelopes={stream.envelopes} streamStatus={stream.status} onFocusGraph={onFocusGraph} />}
        graph={<Suspense fallback={<PanelLoading label="Loading Graph Explorer" />}><GraphExplorer entityHighlights={entityHighlights} relationshipHighlights={relationshipHighlights} focusRevision={focusRevision} /></Suspense>}
        answer={<Suspense fallback={<PanelLoading label="Loading Final Answer" />}><AnswerPanel runId={selectedRunId} result={selected.result} loading={selected.loading} /></Suspense>}
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
