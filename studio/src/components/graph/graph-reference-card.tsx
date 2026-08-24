import { Building2, UsersRound } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { GraphCommunityRef, GraphEntityRef } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { HoverCard, HoverCardContent, HoverCardTrigger } from "@/components/ui/hover-card"

type GraphReference =
  | { kind: "entity"; value: GraphEntityRef }
  | { kind: "community"; value: GraphCommunityRef }

interface GraphReferenceCardProps {
  reference: GraphReference
  onOpen: (id: string) => void
}

export function GraphReferenceCard({ onOpen, reference }: GraphReferenceCardProps): React.ReactElement {
  const { t } = useTranslation()
  const { value } = reference
  const community = reference.kind === "community"
  const secondary = community
    ? `${t("graph.labels.levelValue", { value: reference.value.level })} · ${t("graph.labels.shortIdValue", { value: reference.value.short_id })}`
    : reference.value.entity_type ?? t("graph.labels.untyped")
  const preview = community ? reference.value.summary : reference.value.description
  const ariaLabel = t(community ? "graph.navigation.openCommunity" : "graph.navigation.openEntity", { title: value.title })

  return (
    <HoverCard openDelay={250} closeDelay={100}>
      <HoverCardTrigger asChild>
        <Button
          type="button"
          variant="outline"
          className="h-auto w-full min-w-0 justify-start gap-3 px-3 py-2.5 text-left"
          aria-label={ariaLabel}
          onClick={() => onOpen(value.id)}
        >
          <span className="shrink-0 rounded-full bg-primary/10 p-2 text-primary">{community ? <UsersRound className="size-4" /> : <Building2 className="size-4" />}</span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium">{value.title}</span>
            <span className="block truncate text-[11px] font-normal text-muted-foreground">{secondary}</span>
          </span>
        </Button>
      </HoverCardTrigger>
      <HoverCardContent>
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <p className="break-words text-sm font-semibold">{value.title}</p>
            <p className="mt-0.5 text-[11px] text-muted-foreground">{secondary}</p>
          </div>
          <Badge variant="outline">{community ? t("graph.status.community") : t("graph.status.entity")}</Badge>
        </div>
        {preview === null ? null : <p className="mt-2 max-h-24 overflow-hidden whitespace-pre-wrap break-words text-xs leading-5 text-muted-foreground">{preview}</p>}
      </HoverCardContent>
    </HoverCard>
  )
}

export function UnresolvedGraphReference({ label, value }: { label: string; value: string }): React.ReactElement {
  return (
    <div className="rounded-md border border-dashed px-3 py-2.5 text-left" aria-disabled="true">
      <p className="truncate text-sm font-medium">{value}</p>
      <p className="mt-0.5 text-[11px] text-muted-foreground">{label}</p>
    </div>
  )
}
