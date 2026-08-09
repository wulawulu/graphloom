import { Braces, CircleAlert, DatabaseZap, GitBranch, LifeBuoy, Sparkles } from "lucide-react"

import type { ExplainabilityEnvelope } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
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
  const structuredEntries = Object.entries(event).filter(([key]) => key !== "type" && !["context", "prompt", "response"].includes(key))

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
            {structuredEntries.length > 0 ? (
              <dl className="grid grid-cols-[minmax(7rem,auto)_1fr] gap-x-3 gap-y-1 text-xs">
                {structuredEntries.map(([key, value]) => (
                  <div key={key} className="contents">
                    <dt className="font-mono text-muted-foreground">{key}</dt>
                    <dd className="min-w-0 overflow-hidden text-ellipsis whitespace-pre-wrap">{typeof value === "string" ? value : JSON.stringify(value)}</dd>
                  </div>
                ))}
              </dl>
            ) : null}
            {event.type === "context_completed" && event.context === undefined ? <p className="text-xs text-muted-foreground">Content hidden by explainability mode.</p> : null}
            <details>
              <summary className="cursor-pointer text-xs font-medium text-muted-foreground">Raw JSON</summary>
              <pre className="mt-2 max-h-72 overflow-auto rounded-md bg-muted p-3 font-mono text-[11px] leading-5">{JSON.stringify(event, null, 2)}</pre>
            </details>
          </CollapsibleContent>
        </div>
      </Collapsible>
    </article>
  )
}
