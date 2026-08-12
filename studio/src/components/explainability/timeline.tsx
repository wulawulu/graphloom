import { Activity, Radio } from "lucide-react"

import type { ExplainabilityEnvelope } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import type { StreamStatus } from "@/hooks/use-explainability-stream"

import { TimelineEvent } from "./timeline-event"

interface TimelineProps {
  runId: string | null
  envelopes: ExplainabilityEnvelope[]
  streamStatus: StreamStatus
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

export function Timeline({ runId, envelopes, streamStatus, onFocusGraph }: TimelineProps): React.ReactElement {
  return (
    <section className="flex size-full min-h-0 flex-col" aria-label="Execution trace">
      <header className="flex h-10 shrink-0 items-center justify-between border-b px-3">
        <div className="flex items-center gap-2"><Activity className="size-4 text-primary" /><h2 className="text-xs font-semibold">Execution Trace</h2></div>
        <Badge variant={streamStatus === "open" ? "success" : streamStatus === "reconnecting" ? "warning" : "outline"}>
          <Radio className={streamStatus === "open" ? "animate-pulse" : ""} /> {streamStatus}
        </Badge>
      </header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="space-y-2 p-3">
          {runId === null ? <EmptyTimeline title="No Run selected" detail="Choose a historical Run or submit a new Local Query." /> : null}
          {runId !== null && envelopes.length === 0 ? <EmptyTimeline title="Waiting for explainability" detail={streamStatus === "reconnecting" ? "The live connection is reconnecting. Persisted history will be replayed." : "The Run has not emitted any events yet."} /> : null}
          {envelopes.map((envelope) => <TimelineEvent key={envelope.sequence} envelope={envelope} onFocusGraph={onFocusGraph} />)}
        </div>
      </ScrollArea>
    </section>
  )
}

function EmptyTimeline({ title, detail }: { title: string; detail: string }): React.ReactElement {
  return <div className="flex min-h-52 flex-col items-center justify-center text-center"><Activity className="mb-3 size-8 text-muted-foreground/50" /><p className="text-sm font-medium">{title}</p><p className="mt-1 max-w-sm text-xs text-muted-foreground">{detail}</p></div>
}
