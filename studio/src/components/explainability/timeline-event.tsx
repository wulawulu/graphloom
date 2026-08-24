import { Braces, CircleAlert, DatabaseZap, GitBranch, LifeBuoy, Sparkles } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityEnvelope } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { EventDetail } from "@/components/explainability/details/event-detail"
import { localizedEventSummary } from "@/i18n/presentation"
import type { StudioTranslationKey } from "@/i18n/types"
import { describeEvent, highlightFromEvent, type TimelineCategory } from "@/lib/explainability"

const CATEGORY_KEYS: Readonly<Record<TimelineCategory, StudioTranslationKey>> = {
  lifecycle: "explainability.labels.lifecycle",
  retrieval: "explainability.labels.retrieval",
  graph: "explainability.labels.graph",
  context: "explainability.labels.context",
  llm: "explainability.labels.llm",
  warning: "explainability.labels.warning",
}

interface TimelineEventProps {
  envelope: ExplainabilityEnvelope
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

function CategoryIcon({ category }: { category: TimelineCategory }): React.ReactElement {
  const className = "size-4"
  if (category === "retrieval") return <DatabaseZap className={className} />
  if (category === "graph") return <GitBranch className={className} />
  if (category === "context") return <Braces className={className} />
  if (category === "llm") return <Sparkles className={className} />
  if (category === "warning") return <CircleAlert className={className} />
  return <LifeBuoy className={className} />
}

export function TimelineEvent({ envelope, onFocusGraph }: TimelineEventProps): React.ReactElement {
  const { t } = useTranslation()
  const event = envelope.record.event
  const descriptor = describeEvent(event)
  const summary = localizedEventSummary(t, event)
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
                <span className="text-sm font-medium capitalize">{descriptor.labelKey === null ? descriptor.rawLabel : t(descriptor.labelKey)}</span>
                <Badge variant="outline">{t(CATEGORY_KEYS[descriptor.category])}</Badge>
                <span className="font-mono text-[10px] text-muted-foreground">#{envelope.sequence}</span>
              </div>
              {summary.length > 0 ? <p className="mt-1 text-xs text-muted-foreground">{summary}</p> : null}
              <p className="mt-1 break-all font-mono text-[10px] text-muted-foreground">{t("explainability.status.span")} {envelope.record.span_id}{envelope.record.parent_span_id === undefined ? "" : ` · ${t("explainability.status.parent")} ${envelope.record.parent_span_id}`}</p>
              <time className="mt-1 block text-[10px] text-muted-foreground">{new Date(envelope.record.timestamp).toLocaleTimeString()}</time>
            </div>
            <div className="flex shrink-0 gap-1">
              {highlight !== null ? <Button variant="outline" size="sm" onClick={() => onFocusGraph(envelope)}>{t("runs.actions.focusInGraph")}</Button> : null}
              <CollapsibleTrigger asChild><Button variant="ghost" size="sm">{t("explainability.labels.details")}</Button></CollapsibleTrigger>
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
