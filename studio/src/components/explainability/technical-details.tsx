import type { ExplainabilityEnvelope } from "@/api/types"
import { TimelineEvent } from "@/components/explainability/timeline-event"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"

interface TechnicalDetailsProps {
  rawEvents: ExplainabilityEnvelope[]
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

export function TechnicalDetails({ rawEvents, onFocusGraph }: TechnicalDetailsProps): React.ReactElement {
  return (
    <Collapsible>
      <CollapsibleTrigger asChild><Button variant="ghost" size="sm" className="mt-3 px-1 text-muted-foreground">Technical details · {rawEvents.length} raw events</Button></CollapsibleTrigger>
      <CollapsibleContent className="mt-2 space-y-2 border-t pt-3">
        {rawEvents.map((envelope) => <TimelineEvent key={envelope.sequence} envelope={envelope} onFocusGraph={onFocusGraph} />)}
      </CollapsibleContent>
    </Collapsible>
  )
}
