import { useTranslation } from "react-i18next"

import type { ExplainabilityEnvelope } from "@/api/types"
import { TimelineEvent } from "@/components/explainability/timeline-event"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"

interface TechnicalDetailsProps {
  visible: boolean
  rawEvents: ExplainabilityEnvelope[]
  onFocusGraph: (envelope: ExplainabilityEnvelope) => void
}

export function TechnicalDetails({ visible, rawEvents, onFocusGraph }: TechnicalDetailsProps): React.ReactElement | null {
  const { t } = useTranslation()
  if (!visible || rawEvents.length === 0) return null
  return (
    <Collapsible>
      <CollapsibleTrigger asChild><Button variant="ghost" size="sm" className="mt-3 px-1 text-muted-foreground">{t("explainability.counts.developerDetailsCountRawEvent", { count: rawEvents.length })}</Button></CollapsibleTrigger>
      <CollapsibleContent className="mt-2 space-y-2 border-t pt-3">
        {rawEvents.map((envelope) => <TimelineEvent key={envelope.sequence} envelope={envelope} onFocusGraph={onFocusGraph} />)}
      </CollapsibleContent>
    </Collapsible>
  )
}
