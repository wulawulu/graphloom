import { useEffect, useMemo, useState } from "react"
import { ChevronDown, History, MessageSquareText, Plus } from "lucide-react"

import type { ExplainabilityEnvelope, ExplainabilityRun } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"
import { RunList } from "@/components/runs/run-list"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import type { StreamStatus } from "@/hooks/use-explainability-stream"
import { buildSemanticTimeline, type ExplainabilityRecordView } from "@/lib/semantic-timeline"

interface QaWorkspaceProps {
  runId: string | null
  runStatus: string | undefined
  question: string | null
  answer: React.ReactNode
  composer: React.ReactNode
  envelopes: ExplainabilityEnvelope[]
  streamStatus: StreamStatus
  runs: ExplainabilityRun[]
  historyLoading: boolean
  historyError: string | null
  historyHasMore: boolean
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  onInspectCandidate: (candidate: ExplainabilityRecordView) => void
  onNewQuery: () => void
  onSelectRun: (runId: string) => void
  onRefreshHistory: () => void
  onLoadMoreHistory: () => void
}

export function QaWorkspace(props: QaWorkspaceProps): React.ReactElement {
  const [analysisOpen, setAnalysisOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const semanticTimeline = useMemo(() => buildSemanticTimeline(props.envelopes), [props.envelopes])
  const decisionCount = semanticTimeline.steps.length
  const methodLabel = queryMethodLabel(semanticTimeline.method)

  useEffect(() => setAnalysisOpen(false), [props.runId])

  const selectHistoryRun = (runId: string): void => {
    props.onSelectRun(runId)
    setHistoryOpen(false)
  }

  return (
    <section className="flex size-full min-h-0 flex-col bg-card/20" aria-label="Graph QA workspace">
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-3">
        <div className="flex items-center gap-2"><MessageSquareText className="size-4 text-primary" /><h2 className="text-sm font-semibold">Graph QA</h2><Badge variant="outline">{methodLabel}</Badge></div>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={props.onNewQuery}><Plus /> New Query</Button>
          <Button variant="ghost" size="sm" onClick={() => setHistoryOpen(true)}><History /> History</Button>
        </div>
      </header>

      <ScrollArea className="min-h-0 flex-1">
        {props.runId === null ? (
          <div className="flex min-h-64 items-center justify-center px-6 text-center text-sm text-muted-foreground">Ask a question about the indexed graph.</div>
        ) : (
          <div className="space-y-6 px-4 py-5">
            <section aria-label="Current question">
              <p className="mb-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">You</p>
              <p className="whitespace-pre-wrap text-sm leading-6">{props.question ?? "Query hidden (metadata mode)"}</p>
            </section>
            <section aria-label="GraphLoom answer">
              <p className="mb-2 text-[11px] font-semibold tracking-wide text-primary uppercase">GraphLoom</p>
              <Collapsible open={analysisOpen} onOpenChange={setAnalysisOpen} className="mb-3 border-b pb-2">
                <CollapsibleTrigger asChild>
                  <Button variant="ghost" className="h-9 w-full justify-between px-1" aria-label="Toggle analysis process">
                    <span className="text-xs font-medium">{analysisSummary(decisionCount, props.runStatus, props.streamStatus)}</span>
                    <ChevronDown className={`size-4 transition-transform ${analysisOpen ? "rotate-180" : ""}`} />
                  </Button>
                </CollapsibleTrigger>
                <CollapsibleContent className="pb-2 pt-3">
                  <Timeline embedded runId={props.runId} envelopes={props.envelopes} streamStatus={props.streamStatus} onFocusGraph={props.onFocusGraph} onInspectCandidate={props.onInspectCandidate} />
                </CollapsibleContent>
              </Collapsible>
              {props.answer}
            </section>
          </div>
        )}
      </ScrollArea>

      <div className="shrink-0 border-t bg-background/80 p-3">{props.composer}</div>

      <Sheet open={historyOpen} onOpenChange={setHistoryOpen}>
        <SheetContent className="gap-2">
          <SheetHeader><SheetTitle>Recent Runs</SheetTitle><SheetDescription>Select an independent Graph QA Run.</SheetDescription></SheetHeader>
          <RunList
            runs={props.runs}
            selectedRunId={props.runId}
            loading={props.historyLoading}
            error={props.historyError}
            hasMore={props.historyHasMore}
            onSelect={selectHistoryRun}
            onRefresh={props.onRefreshHistory}
            onLoadMore={props.onLoadMoreHistory}
          />
        </SheetContent>
      </Sheet>
    </section>
  )
}

function analysisSummary(decisionCount: number, runStatus: string | undefined, streamStatus: StreamStatus): string {
  const count = `${decisionCount} ${decisionCount === 1 ? "decision" : "decisions"}`
  if (runStatus === "completed") return `Analysis process · ${count} · completed`
  if (runStatus === "failed" || runStatus === "cancelled") return `Analysis process · ${count} · ${runStatus}`
  if (runStatus === "running" || runStatus === "pending" || streamStatus === "open" || streamStatus === "connecting" || streamStatus === "reconnecting") {
    return `Analysis process · ${count} · running`
  }
  return `Analysis process · ${count}`
}

function queryMethodLabel(method: "local" | "global" | "basic" | "drift" | null): string {
  if (method === "local") return "Local"
  if (method === "global") return "Global"
  if (method === "basic") return "Basic"
  if (method === "drift") return "DRIFT"
  return "Unknown"
}
