import { useLayoutEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

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
  const { t } = useTranslation()
  const tooltipRef = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState({ x: 8, y: 8 })
  useLayoutEffect(() => {
    const element = tooltipRef.current
    if (element === null) return
    setPosition(clampTooltipPosition(x, y, bounds.width, bounds.height, element.offsetWidth, element.offsetHeight))
  }, [bounds.height, bounds.width, content, x, y])
  return (
    <div
      ref={tooltipRef}
      className="pointer-events-none absolute z-30 max-h-[calc(100%-1rem)] overflow-hidden rounded-md border bg-popover px-3 py-2 text-popover-foreground shadow-xl"
      style={{ left: position.x, top: position.y, maxWidth: Math.max(0, bounds.width - 16) }}
      role="tooltip"
    >
      {content.kind === "entity" ? (
        <><p className="font-semibold">{content.value.title}</p><p className="mt-0.5 text-xs text-muted-foreground">{content.value.entity_type ?? t("graph.labels.untyped")}</p><p className="mt-1 text-xs text-muted-foreground">{t("graph.labels.degreeValue", { value: content.value.degree ?? "—" })} · {t("graph.labels.rankValue", { value: content.value.rank ?? "—" })}</p></>
      ) : (
        <><p className="font-semibold">{content.value.source} → {content.value.target}</p><p className="mt-0.5 text-xs text-muted-foreground">{t("graph.labels.weightValue", { value: content.value.weight ?? "—" })} · {t("graph.labels.rankValue", { value: content.value.rank ?? "—" })}</p></>
      )}
      <p className="mt-1.5 text-[10px] text-muted-foreground">{t("graph.labels.clickForDetails")}</p>
    </div>
  )
}
