import { useMemo } from "react"
import { Activity, Radio } from "lucide-react"

import type { ExplainabilityEnvelope } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import type { StreamStatus } from "@/hooks/use-explainability-stream"
import { buildSemanticTimeline, type ExplainabilityRecordView } from "@/lib/semantic-timeline"
import { isGlobalSemanticStep } from "@/lib/semantic-global"

import { GlobalSemanticStepCard } from "./global-semantic-step"
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
  const model = useMemo(() => buildSemanticTimeline(envelopes), [envelopes])
  const content = (
    <div className="space-y-2 p-3">
      {runId === null ? <EmptyTimeline title="No Run selected" detail="Choose a historical Run or submit a new Query." /> : null}
      {runId !== null && envelopes.length === 0 ? <EmptyTimeline title="Waiting for explainability" detail={streamStatus === "reconnecting" ? "The live connection is reconnecting. Persisted history will be replayed." : "The Run has not emitted any events yet."} /> : null}
      {model.steps.map((step) => isGlobalSemanticStep(step)
        ? <GlobalSemanticStepCard key={`${runId ?? "none"}:${step.id}`} step={step} onFocusGraph={onFocusGraph} />
        : <SemanticStepCard key={`${runId ?? "none"}:${step.id}`} step={step} onFocusGraph={onFocusGraph} onInspectCandidate={onInspectCandidate} />)}
      {model.diagnosticEvents.length > 0 ? <details className="rounded-md border bg-muted/20 p-3"><summary className="cursor-pointer text-xs font-medium text-muted-foreground">Diagnostics / Raw events · {model.diagnosticEvents.length}</summary><div className="mt-3 space-y-2">{model.diagnosticEvents.map((envelope) => <TimelineEvent key={envelope.sequence} envelope={envelope} onFocusGraph={onFocusGraph} />)}</div></details> : null}
    </div>
  )
  if (embedded) return <section aria-label="Analysis process">{content}</section>
  return (
    <section className="flex size-full min-h-0 flex-col" aria-label="Decision timeline">
      <header className="flex h-10 shrink-0 items-center justify-between border-b px-3">
        <div className="flex items-center gap-2"><Activity className="size-4 text-primary" /><h2 className="text-xs font-semibold">Decision Timeline</h2></div>
        <Badge variant={streamStatus === "open" ? "success" : streamStatus === "reconnecting" ? "warning" : "outline"}>
          <Radio className={streamStatus === "open" ? "animate-pulse" : ""} /> {streamStatus}
        </Badge>
      </header>
      <ScrollArea className="min-h-0 flex-1">
        {content}
      </ScrollArea>
    </section>
  )
}

function EmptyTimeline({ title, detail }: { title: string; detail: string }): React.ReactElement {
  return <div className="flex min-h-52 flex-col items-center justify-center text-center"><Activity className="mb-3 size-8 text-muted-foreground/50" /><p className="text-sm font-medium">{title}</p><p className="mt-1 max-w-sm text-xs text-muted-foreground">{detail}</p></div>
}
