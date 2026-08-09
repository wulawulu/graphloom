import { Braces, CircleAlert, DatabaseZap, GitBranch, LifeBuoy, Sparkles } from "lucide-react"

import type { ExplainabilityEnvelope } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { EventDetail } from "@/components/explainability/details/event-detail"
import { describeEvent, eventSummary, highlightFromEvent, type TimelineCategory } from "@/lib/explainability"

interface TimelineEventProps {
  envelope: ExplainabilityEnvelope
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

function CategoryIcon({ category }: { category: TimelineCategory }): React.ReactElement {
  const className = "size-4"
  if (category === "Retrieval") return <DatabaseZap className={className} />
  if (category === "Graph") return <GitBranch className={className} />
  if (category === "Context") return <Braces className={className} />
  if (category === "LLM") return <Sparkles className={className} />
  if (category === "Warning") return <CircleAlert className={className} />
  return <LifeBuoy className={className} />
}

export function TimelineEvent({ envelope, onFocusGraph }: TimelineEventProps): React.ReactElement {
  const event = envelope.record.event
  const descriptor = describeEvent(event)
  const summary = eventSummary(event)
  const highlight = highlightFromEvent(event)

  return (
    <article className="relative pl-8">
      <div className="absolute top-0 left-0 flex size-6 items-center justify-center rounded-full border bg-card text-primary"><CategoryIcon category={descriptor.category} /></div>
      <div className="absolute top-6 bottom-[-12px] left-[11px] w-px bg-border last:hidden" />
      <Collapsible>
        <div className="rounded-md border bg-card/70 p-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium capitalize">{descriptor.label}</span>
                <Badge variant="outline">{descriptor.category}</Badge>
                <span className="font-mono text-[10px] text-muted-foreground">#{envelope.sequence}</span>
              </div>
              {summary.length > 0 ? <p className="mt-1 text-xs text-muted-foreground">{summary}</p> : null}
              <time className="mt-1 block text-[10px] text-muted-foreground">{new Date(envelope.record.timestamp).toLocaleTimeString()}</time>
            </div>
            <div className="flex shrink-0 gap-1">
              {highlight !== null ? <Button variant="outline" size="sm" onClick={() => onFocusGraph(envelope)}>Focus in graph</Button> : null}
              <CollapsibleTrigger asChild><Button variant="ghost" size="sm">Details</Button></CollapsibleTrigger>
            </div>
          </div>
          <CollapsibleContent className="mt-3 space-y-3 border-t pt-3">
            <EventDetail event={event} onFocusGraph={highlight === null ? null : () => onFocusGraph(envelope)} />
          </CollapsibleContent>
        </div>
      </Collapsible>
    </article>
  )
}
