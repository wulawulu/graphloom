import { useEffect, useMemo, useRef } from "react"
import type { Core } from "cytoscape"
import { Focus, RefreshCcw } from "lucide-react"

import type { GraphEntity, GraphRelationship } from "@/api/types"
import { Button } from "@/components/ui/button"
import { applyGraphHighlights, buildGraphElements } from "@/lib/graph"

interface NetworkPreviewProps {
  entities: GraphEntity[]
  relationships: GraphRelationship[]
  totalEntities: number
  entityHighlights: ReadonlySet<string>
  relationshipHighlights: ReadonlySet<string>
  onEntity: (id: string) => void
  onRelationship: (id: string) => void
}

export function NetworkPreview(props: NetworkPreviewProps): React.ReactElement {
  const { entities, relationships, onEntity, onRelationship } = props
  const containerRef = useRef<HTMLDivElement>(null)
  const cytoscapeRef = useRef<Core | null>(null)
  const highlightRef = useRef({ entityIds: props.entityHighlights, relationshipIds: props.relationshipHighlights })
  const preview = useMemo(() => buildGraphElements(entities, relationships), [entities, relationships])

  useEffect(() => {
    const container = containerRef.current
    if (container === null || container.clientWidth === 0) return undefined
    let disposed = false
    let instance: Core | null = null
    void import("cytoscape").then(({ default: cytoscape }) => {
      if (disposed) return
      const styles = getComputedStyle(document.documentElement)
      const cy = cytoscape({
        container,
        elements: preview.elements,
        layout: { name: "cose", animate: false, randomize: true, nodeRepulsion: () => 8_000 },
        style: [
          { selector: "node", style: { "background-color": styles.getPropertyValue("--primary").trim(), "border-color": styles.getPropertyValue("--background").trim(), "border-width": 2, label: "data(label)", color: styles.getPropertyValue("--foreground").trim(), "font-size": 9, "text-outline-color": styles.getPropertyValue("--background").trim(), "text-outline-width": 2, width: 24, height: 24 } },
          { selector: "edge", style: { width: 1.5, "line-color": styles.getPropertyValue("--border").trim(), "target-arrow-color": styles.getPropertyValue("--border").trim(), "target-arrow-shape": "triangle", "curve-style": "bezier" } },
          { selector: ".highlighted", style: { "background-color": styles.getPropertyValue("--warning").trim(), "line-color": styles.getPropertyValue("--warning").trim(), "target-arrow-color": styles.getPropertyValue("--warning").trim(), width: 5, "border-width": 3 } },
        ],
        minZoom: 0.2,
        maxZoom: 3,
      })
      cy.on("tap", "node", (event) => onEntity(event.target.id()))
      cy.on("tap", "edge", (event) => onRelationship(event.target.id()))
      instance = cy
      cytoscapeRef.current = cy
      applyGraphHighlights(cy, highlightRef.current.entityIds, highlightRef.current.relationshipIds)
    }).catch(() => undefined)
    return () => {
      disposed = true
      if (cytoscapeRef.current === instance) cytoscapeRef.current = null
      instance?.destroy()
    }
  }, [onEntity, onRelationship, preview.elements])

  useEffect(() => {
    highlightRef.current = { entityIds: props.entityHighlights, relationshipIds: props.relationshipHighlights }
    const cy = cytoscapeRef.current
    if (cy === null) return
    applyGraphHighlights(cy, props.entityHighlights, props.relationshipHighlights)
  }, [props.entityHighlights, props.relationshipHighlights])

  const outsideHighlights = [...props.entityHighlights].filter((id) => !props.entities.some((entity) => entity.id === id)).length

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <p className="text-[11px] text-muted-foreground">
          Previewing {props.entities.length} of {props.totalEntities} entities · {preview.resolvedRelationshipCount} relationships
          {preview.omittedRelationshipCount > 0 ? ` · ${preview.omittedRelationshipCount} unresolved/ambiguous edges omitted` : ""}
          {outsideHighlights > 0 ? ` · ${outsideHighlights} selected entities outside preview` : ""}
        </p>
        <div className="flex gap-1">
          <Button variant="ghost" size="icon" aria-label="Fit graph preview" onClick={() => cytoscapeRef.current?.fit(undefined, 24)}><Focus /></Button>
          <Button variant="ghost" size="icon" aria-label="Re-layout graph preview" onClick={() => cytoscapeRef.current?.layout({ name: "cose", animate: true }).run()}><RefreshCcw /></Button>
        </div>
      </div>
      <div ref={containerRef} className="min-h-80 flex-1 rounded-md border bg-background" aria-label="Knowledge graph network preview" />
    </div>
  )
}
