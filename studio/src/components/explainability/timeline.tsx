import { useMemo } from "react"
import { Activity, Radio } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityEnvelope } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import type { StreamStatus } from "@/hooks/use-explainability-stream"
import type { StudioTranslationKey } from "@/i18n/types"
import { buildSemanticTimeline, type ExplainabilityRecordView } from "@/lib/semantic-timeline"
import { isBasicSemanticStep } from "@/lib/semantic-basic"
import { isGlobalSemanticStep } from "@/lib/semantic-global"
import { isDriftSemanticStep } from "@/lib/semantic-drift"
import { classifyTimelineResidualEvents, detectRunContentMode } from "@/lib/timeline-presentation"
import { BasicSemanticStepCard } from "./basic-semantic-step"
import { GlobalSemanticStepCard } from "./global-semantic-step"
import { DriftSemanticStepCard } from "./drift-semantic-step"
import { SemanticStepCard } from "./semantic-step"
import { TimelineEvent } from "./timeline-event"

interface TimelineProps {
  embedded?: boolean
  runId: string | null
  envelopes: ExplainabilityEnvelope[]
  streamStatus: StreamStatus
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
  onInspectCandidate: (candidate: ExplainabilityRecordView) => void
}

export function Timeline({ embedded = false, runId, envelopes, streamStatus, onFocusGraph, onInspectCandidate }: TimelineProps): React.ReactElement {
  const { t } = useTranslation()
  const model = useMemo(() => buildSemanticTimeline(envelopes), [envelopes])
  const contentMode = detectRunContentMode(envelopes)
  const showDeveloperDetails = contentMode === "debug"
  const residual = classifyTimelineResidualEvents(model.diagnosticEvents)
  const content = (
    <div className="space-y-2 p-3">
      {runId === null ? <EmptyTimeline title={t("runs.labels.noRunSelected")} detail={t("runs.messages.chooseAHistoricalRunOrSubmitANewQuery")} /> : null}
      {runId !== null && envelopes.length === 0 ? <EmptyTimeline title={t("runs.labels.waitingForExplainability")} detail={streamStatus === "reconnecting" ? t("runs.messages.replayOnReconnect") : t("explainability.empty.noEvents")} /> : null}
      {model.steps.map((step) => isGlobalSemanticStep(step)
        ? <GlobalSemanticStepCard key={`${runId ?? "none"}:${step.id}`} step={step} onFocusGraph={onFocusGraph} showDeveloperDetails={showDeveloperDetails} />
        : isDriftSemanticStep(step)
          ? <DriftSemanticStepCard key={`${runId ?? "none"}:${step.id}`} step={step} onFocusGraph={onFocusGraph} showDeveloperDetails={showDeveloperDetails} />
        : isBasicSemanticStep(step)
          ? <BasicSemanticStepCard key={`${runId ?? "none"}:${step.id}`} step={step} onFocusGraph={onFocusGraph} showDeveloperDetails={showDeveloperDetails} />
          : <SemanticStepCard key={`${runId ?? "none"}:${step.id}`} step={step} onFocusGraph={onFocusGraph} onInspectCandidate={onInspectCandidate} showDeveloperDetails={showDeveloperDetails} />)}
      {residual.warnings.length > 0 ? <details className="rounded-md border border-warning/40 bg-warning/5 p-3" role="alert"><summary className="cursor-pointer text-xs font-medium">{t("explainability.counts.warningsCount", { count: residual.warnings.length })}</summary><div className="mt-3 space-y-2">{residual.warnings.map((envelope) => <div key={envelope.sequence} className="space-y-2"><WarningSummary envelope={envelope} /><TimelineEvent envelope={envelope} onFocusGraph={onFocusGraph} /></div>)}</div></details> : null}
      {showDeveloperDetails && residual.developerEvents.length > 0 ? <details className="rounded-md border bg-muted/20 p-3"><summary className="cursor-pointer text-xs font-medium text-muted-foreground">{t("explainability.counts.developerEventsCount", { count: residual.developerEvents.length })}</summary><div className="mt-3 space-y-2">{residual.developerEvents.map((envelope) => <TimelineEvent key={envelope.sequence} envelope={envelope} onFocusGraph={onFocusGraph} />)}</div></details> : null}
    </div>
  )
  if (embedded) return <section aria-label={t("runs.labels.analysisProcess")}>{content}</section>
  return (
    <section className="flex size-full min-h-0 flex-col" aria-label={t("explainability.labels.decisionTimeline")}>
      <header className="flex h-10 shrink-0 items-center justify-between border-b px-3">
        <div className="flex items-center gap-2"><Activity className="size-4 text-primary" /><h2 className="text-xs font-semibold">{t("explainability.labels.decisionTimelineHeading")}</h2></div>
        <Badge variant={streamStatus === "open" ? "success" : streamStatus === "reconnecting" ? "warning" : "outline"}>
          <Radio className={streamStatus === "open" ? "animate-pulse" : ""} /> {t(streamStatusKey(streamStatus))}
        </Badge>
      </header>
      <ScrollArea className="min-h-0 flex-1">
        {content}
      </ScrollArea>
    </section>
  )
}

function WarningSummary({ envelope }: { envelope: ExplainabilityEnvelope }): React.ReactElement | null {
  const event = envelope.record.event
  const code = typeof event.code === "string" ? event.code : typeof event.error_kind === "string" ? event.error_kind : null
  const message = typeof event.message === "string" ? event.message : null
  if (code === null && message === null) return null
  return <p className="break-words text-xs"><span className="font-mono font-medium">{code}</span>{code === null || message === null ? "" : " · "}{message}</p>
}

function streamStatusKey(status: StreamStatus): StudioTranslationKey {
  return `answer.status.${status}`
}

function EmptyTimeline({ title, detail }: { title: string; detail: string }): React.ReactElement {
  return <div className="flex min-h-52 flex-col items-center justify-center text-center"><Activity className="mb-3 size-8 text-muted-foreground/50" /><p className="text-sm font-medium">{title}</p><p className="mt-1 max-w-sm text-xs text-muted-foreground">{detail}</p></div>
}
