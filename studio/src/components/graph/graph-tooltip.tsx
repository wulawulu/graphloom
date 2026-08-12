import type { GraphProjectionEntity, GraphProjectionRelationship } from "@/api/types"
import { clampTooltipPosition } from "@/lib/graph-tooltip"

export type GraphTooltipContent =
  | { kind: "entity"; value: GraphProjectionEntity }
  | { kind: "relationship"; value: GraphProjectionRelationship }

interface GraphTooltipProps {
  content: GraphTooltipContent
  x: number
  y: number
  bounds: { width: number; height: number }
}

export function GraphTooltip({ bounds, content, x, y }: GraphTooltipProps): React.ReactElement {
  const position = clampTooltipPosition(x, y, bounds.width, bounds.height)
  return (
    <div
      className="pointer-events-none absolute z-30 max-w-64 rounded-md border bg-popover px-3 py-2 text-popover-foreground shadow-xl"
      style={{ left: position.x, top: position.y }}
      role="tooltip"
    >
      {content.kind === "entity" ? (
        <><p className="font-semibold">{content.value.title}</p><p className="mt-0.5 text-xs text-muted-foreground">{content.value.entity_type ?? "Untyped"} · Rank {content.value.rank ?? "—"}</p></>
      ) : (
        <><p className="font-semibold">{content.value.source} → {content.value.target}</p><p className="mt-0.5 text-xs text-muted-foreground">Weight {content.value.weight ?? "—"} · Rank {content.value.rank ?? "—"}</p></>
      )}
      <p className="mt-1.5 text-[10px] text-muted-foreground">Click for details</p>
    </div>
  )
}
