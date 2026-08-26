import { useEffect, useMemo, useRef, useState } from "react"
import { ArrowDown, ChevronDown, History, MessageSquareText, Plus } from "lucide-react"
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
import { useAutoFollow } from "@/hooks/use-auto-follow"
import type { StreamStatus } from "@/hooks/use-explainability-stream"
import type { StudioTranslationKey } from "@/i18n/types"
import type { TFunction } from "i18next"
import { buildSemanticTimeline, type ExplainabilityRecordView } from "@/lib/semantic-timeline"

interface QaWorkspaceProps {
  runId: string | null
  runStatus: string | undefined
  question: string | null
  answer: React.ReactNode
  answerHasStarted: boolean
  isActiveSubmission: boolean
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
  const [analysisUserControlled, setAnalysisUserControlled] = useState(false)
  const analysisRunId = useRef<string | null>(null)
  const analysisInitialized = useRef(false)
  const answerStartedRunId = useRef<string | null>(null)
  const previousAnswerHasStarted = useRef(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const { viewportRef, contentRef, following, pause, resume, scrollToLatest } = useAutoFollow({ resetKey: props.runId })
  const semanticTimeline = useMemo(() => buildSemanticTimeline(props.envelopes), [props.envelopes])
  const decisionCount = semanticTimeline.steps.length
  const methodLabel = t(queryMethodLabel(semanticTimeline.method, semanticTimeline.globalVariant))

  useEffect(() => {
    if (analysisRunId.current !== props.runId) {
      analysisRunId.current = props.runId
      analysisInitialized.current = false
      setAnalysisUserControlled(false)
    }
    if (analysisInitialized.current || (!props.isActiveSubmission && props.runStatus === undefined)) return
    analysisInitialized.current = true
    setAnalysisOpen(props.isActiveSubmission || props.runStatus === "running" || props.runStatus === "pending")
  }, [props.isActiveSubmission, props.runId, props.runStatus])

  useEffect(() => {
    if (answerStartedRunId.current !== props.runId) {
      answerStartedRunId.current = props.runId
      previousAnswerHasStarted.current = props.answerHasStarted
      return
    }
    const firstAnswerTokenArrived = !previousAnswerHasStarted.current && props.answerHasStarted
    previousAnswerHasStarted.current = props.answerHasStarted
    if (firstAnswerTokenArrived && props.isActiveSubmission && !analysisUserControlled) setAnalysisOpen(false)
  }, [analysisUserControlled, props.answerHasStarted, props.isActiveSubmission, props.runId])

  const selectHistoryRun = (runId: string): void => {
    props.onSelectRun(runId)
    setHistoryOpen(false)
  }

  const startNewQuery = (): void => {
    resume()
    props.onNewQuery()
  }

  const setAnalysisFromUser = (open: boolean): void => {
    if (open && !analysisOpen) pause()
    setAnalysisOpen(open)
    setAnalysisUserControlled(true)
  }

  return (
    <section className="flex size-full min-h-0 flex-col bg-card/20" aria-label={t("answer.labels.graphQaWorkspace")}>
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-3">
        <div className="flex min-w-0 items-center gap-2"><MessageSquareText className="size-4 shrink-0 text-primary" /><h2 className="truncate text-sm font-semibold">{t("answer.labels.graphQa")}</h2><Badge variant="outline" className="max-w-36 truncate">{methodLabel}</Badge></div>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={startNewQuery}><Plus /> {t("answer.labels.newQuery")}</Button>
          <Button variant="ghost" size="sm" onClick={() => setHistoryOpen(true)}><History /> {t("answer.labels.history")}</Button>
        </div>
      </header>

      <div className="relative min-h-0 flex-1">
        <ScrollArea viewportRef={viewportRef} className="size-full">
          <div ref={contentRef}>
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
                    <Collapsible open={analysisOpen} onOpenChange={setAnalysisFromUser} className="mb-3 border-b pb-2">
                      <CollapsibleTrigger asChild>
                        <Button variant="ghost" className="h-9 w-full justify-between px-1" aria-label={t("answer.actions.toggleAnalysisProcess")}>
                          <span className="min-w-0 truncate text-xs font-medium">{analysisSummary(t, decisionCount, props.runStatus, props.streamStatus, props.answerHasStarted)}</span>
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
          </div>
        </ScrollArea>
        {props.runId !== null && !following ? (
          <div className="pointer-events-none absolute inset-x-0 bottom-3 z-10 flex justify-end px-4">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="pointer-events-auto bg-background/95 shadow-sm backdrop-blur-sm"
              aria-label={t("answer.actions.backToLatest")}
              onClick={scrollToLatest}
            >
              <ArrowDown /> {t("answer.actions.backToLatest")}
            </Button>
          </div>
        ) : null}
      </div>

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

function analysisSummary(t: TFunction, decisionCount: number, runStatus: string | undefined, streamStatus: StreamStatus, answerHasStarted: boolean): string {
  if (runStatus === "completed") return t("answer.counts.analysisStepsCompleted", { count: decisionCount })
  if (runStatus === "failed" || runStatus === "cancelled") return t("answer.counts.analysisStepsInterrupted", { count: decisionCount })
  if (answerHasStarted) return t("answer.counts.generatingAnswerSteps", { count: decisionCount })
  if (runStatus === "running" || runStatus === "pending" || streamStatus === "open" || streamStatus === "connecting" || streamStatus === "reconnecting") {
    return t("answer.counts.analysisStepsRunning", { count: decisionCount })
  }
  return t("answer.counts.analysisSteps", { count: decisionCount })
}

function queryMethodLabel(method: "local" | "global" | "basic" | "drift" | null, globalVariant: "static" | "dynamic" | null): StudioTranslationKey {
  if (method === "local") return "query.methods.local"
  if (method === "global") return globalVariant === "dynamic" ? "query.methods.dynamicGlobal" : "query.methods.global"
  if (method === "basic") return "query.methods.basic"
  if (method === "drift") return "query.methods.drift"
  return "graph.labels.unknown"
}
