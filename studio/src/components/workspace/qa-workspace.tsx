import { useEffect, useMemo, useState } from "react"
import { ChevronDown, History, MessageSquareText, Plus } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityEnvelope, ExplainabilityRun } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"
import { TextUnitEvidenceProvider } from "@/contexts/text-unit-evidence-provider"
import { RunList } from "@/components/runs/run-list"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import type { StreamStatus } from "@/hooks/use-explainability-stream"
import type { StudioTranslationKey } from "@/i18n/types"
import type { TFunction } from "i18next"
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
  historyError: StudioTranslationKey | null
  historyHasMore: boolean
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  onInspectCandidate: (candidate: ExplainabilityRecordView) => void
  onNewQuery: () => void
  onSelectRun: (runId: string) => void
  onRefreshHistory: () => void
  onLoadMoreHistory: () => void
}

export function QaWorkspace(props: QaWorkspaceProps): React.ReactElement {
  const { t } = useTranslation()
  const [analysisOpen, setAnalysisOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const semanticTimeline = useMemo(() => buildSemanticTimeline(props.envelopes), [props.envelopes])
  const decisionCount = semanticTimeline.steps.length
  const methodLabel = t(queryMethodLabel(semanticTimeline.method, semanticTimeline.globalVariant))

  useEffect(() => setAnalysisOpen(false), [props.runId])

  const selectHistoryRun = (runId: string): void => {
    props.onSelectRun(runId)
    setHistoryOpen(false)
  }

  return (
    <section className="flex size-full min-h-0 flex-col bg-card/20" aria-label={t("answer.labels.graphQaWorkspace")}>
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-3">
        <div className="flex min-w-0 items-center gap-2"><MessageSquareText className="size-4 shrink-0 text-primary" /><h2 className="truncate text-sm font-semibold">{t("answer.labels.graphQa")}</h2><Badge variant="outline" className="max-w-36 truncate">{methodLabel}</Badge></div>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={props.onNewQuery}><Plus /> {t("answer.labels.newQuery")}</Button>
          <Button variant="ghost" size="sm" onClick={() => setHistoryOpen(true)}><History /> {t("answer.labels.history")}</Button>
        </div>
      </header>

      <ScrollArea className="min-h-0 flex-1">
        {props.runId === null ? (
          <div className="flex min-h-64 items-center justify-center px-6 text-center text-sm text-muted-foreground">{t("answer.messages.askAQuestionAboutTheIndexedGraph")}</div>
        ) : (
          <div className="space-y-6 px-4 py-5">
            <section aria-label={t("answer.labels.currentQuestion")}>
              <p className="mb-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">{t("answer.labels.you")}</p>
              <p className="whitespace-pre-wrap text-sm leading-6">{props.question ?? t("query.labels.queryHiddenMetadataMode")}</p>
            </section>
            <section aria-label={t("answer.labels.graphLoomAnswer")}>
              <p className="mb-2 text-[11px] font-semibold tracking-wide text-primary uppercase">GraphLoom</p>
              <TextUnitEvidenceProvider key={props.runId ?? "no-run"}>
                <Collapsible open={analysisOpen} onOpenChange={setAnalysisOpen} className="mb-3 border-b pb-2">
                  <CollapsibleTrigger asChild>
                    <Button variant="ghost" className="h-9 w-full justify-between px-1" aria-label={t("answer.actions.toggleAnalysisProcess")}>
                      <span className="min-w-0 truncate text-xs font-medium">{analysisSummary(t, decisionCount, props.runStatus, props.streamStatus)}</span>
                      <ChevronDown className={`size-4 transition-transform ${analysisOpen ? "rotate-180" : ""}`} />
                    </Button>
                  </CollapsibleTrigger>
                  <CollapsibleContent className="pb-2 pt-3">
                    <Timeline embedded runId={props.runId} envelopes={props.envelopes} streamStatus={props.streamStatus} onFocusGraph={props.onFocusGraph} onInspectCandidate={props.onInspectCandidate} />
                  </CollapsibleContent>
                </Collapsible>
                {props.answer}
              </TextUnitEvidenceProvider>
            </section>
          </div>
        )}
      </ScrollArea>

      <div className="shrink-0 border-t bg-background/80 p-3">{props.composer}</div>

      <Sheet open={historyOpen} onOpenChange={setHistoryOpen}>
        <SheetContent className="gap-2">
          <SheetHeader><SheetTitle>{t("answer.labels.recentRuns")}</SheetTitle><SheetDescription>{t("answer.messages.selectAnIndependentGraphQaRun")}</SheetDescription></SheetHeader>
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

function analysisSummary(t: TFunction, decisionCount: number, runStatus: string | undefined, streamStatus: StreamStatus): string {
  const status = runStatus === "failed" ? t("answer.status.failed") : runStatus === "cancelled" ? t("answer.status.cancelled") : null
  if (runStatus === "completed") return t("answer.counts.analysisProcessCountDecisionCompleted", { count: decisionCount })
  if (status !== null) return t("answer.counts.analysisProcessCountDecisionStatus", { count: decisionCount, status })
  if (runStatus === "running" || runStatus === "pending" || streamStatus === "open" || streamStatus === "connecting" || streamStatus === "reconnecting") {
    return t("answer.counts.analysisProcessCountDecisionRunning", { count: decisionCount })
  }
  return t("answer.counts.analysisProcessCountDecision", { count: decisionCount })
}

function queryMethodLabel(method: "local" | "global" | "basic" | "drift" | null, globalVariant: "static" | "dynamic" | null): StudioTranslationKey {
  if (method === "local") return "query.methods.local"
  if (method === "global") return globalVariant === "dynamic" ? "query.methods.dynamicGlobal" : "query.methods.global"
  if (method === "basic") return "query.methods.basic"
  if (method === "drift") return "query.methods.drift"
  return "graph.labels.unknown"
}
