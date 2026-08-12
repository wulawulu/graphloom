import { useEffect, useMemo, useRef, useState } from "react"
import type { Core } from "cytoscape"
import { Focus, LoaderCircle, RefreshCcw, RotateCw } from "lucide-react"

import type { GraphProjection, GraphSummary } from "@/api/types"
import type { GraphViewMode } from "@/components/graph/graph-explorer"
import { GraphTooltip, type GraphTooltipContent } from "@/components/graph/graph-tooltip"
import { Button } from "@/components/ui/button"
import { readCytoscapeTheme } from "@/lib/cytoscape-theme"
import { buildProjectionElements, GRAPH_LAYOUT_OPTIONS, graphViewportAction, projectionFocusIds } from "@/lib/graph"

interface NetworkPreviewProps {
  projection: GraphProjection
  summary: GraphSummary | null
  summaryError: boolean
  mode: GraphViewMode
  loading: boolean
  error: string | null
  onEntity: (id: string) => void
  onRelationship: (id: string) => void
  onBackOverview: () => void
  onReload: () => void
}

function focusCollection(cy: Core, projection: GraphProjection, mode: GraphViewMode): void {
  const focused = mode !== "overview"
  const ids = new Set(projectionFocusIds(projection, focused))
  const targets = cy.elements().filter((element) => ids.has(element.id()))
  if (targets.length === 0) return
  if (focused) {
    const animation = {
      // Cytoscape's public fit option spells this element collection key as two joined fragments.
      fit: { ["el" + "es"]: targets, padding: 56 },
      duration: 350,
    } as unknown as Parameters<Core["animate"]>[0]
    cy.animate(animation)
  } else {
    cy.fit(targets, 64)
  }
}

function runLayout(cy: Core, projection: GraphProjection, mode: GraphViewMode, animate: boolean): void {
  const options = { ...GRAPH_LAYOUT_OPTIONS, animate } as unknown as Parameters<Core["layout"]>[0]
  const layout = cy.layout(options)
  layout.one("layoutstop", () => focusCollection(cy, projection, mode))
  layout.run()
}

export function NetworkPreview(props: NetworkPreviewProps): React.ReactElement {
  const { mode, onEntity, onRelationship, projection } = props
  const containerRef = useRef<HTMLDivElement>(null)
  const cytoscapeRef = useRef<Core | null>(null)
  const onEntityRef = useRef(onEntity)
  const onRelationshipRef = useRef(onRelationship)
  const [rendererError, setRendererError] = useState(false)
  const [tooltip, setTooltip] = useState<{ content: GraphTooltipContent; x: number; y: number; bounds: { width: number; height: number } } | null>(null)
  const elements = useMemo(() => buildProjectionElements(projection), [projection])
  const entities = useMemo(() => new Map(projection.entities.map((entity) => [entity.id, entity])), [projection.entities])
  const relationships = useMemo(() => new Map(projection.relationships.map((relationship) => [relationship.id, relationship])), [projection.relationships])

  useEffect(() => { onEntityRef.current = onEntity }, [onEntity])
  useEffect(() => { onRelationshipRef.current = onRelationship }, [onRelationship])

  useEffect(() => {
    const container = containerRef.current
    if (container === null) return undefined
    let disposed = false
    let initializing = false
    let failed = false
    let instance: Core | null = null

    const failInitialization = (error: unknown): void => {
      initializing = false
      failed = true
      instance?.destroy()
      instance = null
      cytoscapeRef.current = null
      console.error("Graph visualization failed to initialize", error)
      if (!disposed) setRendererError(true)
    }

    const initialize = (): void => {
      if (disposed || failed || initializing || instance !== null) return
      initializing = true
      setRendererError(false)
      void import("cytoscape").then(({ default: cytoscape }) => {
        if (disposed) return
        const theme = readCytoscapeTheme(getComputedStyle(document.documentElement))
        const cy = cytoscape({
        container,
        elements,
        layout: { name: "preset" },
        style: [
          { selector: "node", style: { "background-color": theme.defaultNode, "border-color": theme.background, "border-width": 2, label: "data(label)", color: theme.foreground, "font-size": 10, shape: "roundrectangle", width: "data(displayWidth)", height: "data(displayHeight)", "text-wrap": "ellipsis", "text-max-width": "data(textMaxWidth)", "text-halign": "center", "text-valign": "center", "text-justification": "center", "z-index": 10 } },
          { selector: 'node[entityType = "PERSON"]', style: { "background-color": theme.person } },
          { selector: 'node[entityType = "ORGANIZATION"], node[entityType = "ORG"]', style: { "background-color": theme.organization } },
          { selector: 'node[entityType = "GEO"]', style: { "background-color": theme.geo } },
          { selector: 'node[entityType = "EVENT"]', style: { "background-color": theme.event } },
          { selector: "node.hovered", style: { "border-width": 3 } },
          { selector: "node.seed", style: { "font-size": 11, "border-color": theme.seed, "border-width": 4 } },
          { selector: "node.ui-selected", style: { "border-color": theme.foreground, "border-style": "double", "border-width": 4 } },
          { selector: "edge", style: { width: 1.2, opacity: 0.48, "line-color": theme.border, "target-arrow-color": theme.border, "target-arrow-shape": "triangle", "arrow-scale": 0.58, "curve-style": "bezier" } },
          { selector: "edge.seed-relationship", style: { width: 3.5, opacity: 0.95, "line-color": theme.seed, "target-arrow-color": theme.seed, "arrow-scale": 0.9, "z-index": 8 } },
          { selector: "edge.hovered, edge.ui-selected", style: { label: "data(sourceLabel) → data(targetLabel)", color: theme.foreground, "font-size": 9, "text-background-color": theme.background, "text-background-opacity": 0.92, "text-background-padding": "3px", "text-background-shape": "roundrectangle", "z-index": 12 } },
          { selector: "edge.ui-selected", style: { width: 3, opacity: 1, "line-color": theme.foreground, "target-arrow-color": theme.foreground } },
        ],
        minZoom: 0.2,
        maxZoom: 3,
        userZoomingEnabled: true,
        userPanningEnabled: true,
        })
        const showTooltip = (content: GraphTooltipContent, event: { renderedPosition?: { x: number; y: number } }): void => {
          const position = event.renderedPosition ?? { x: 0, y: 0 }
          setTooltip({ content, x: position.x, y: position.y, bounds: { width: container.clientWidth, height: container.clientHeight } })
        }
        cy.on("mouseover", "node", (event) => {
          event.target.addClass("hovered")
          const entity = entities.get(event.target.id())
          if (entity !== undefined) showTooltip({ kind: "entity", value: entity }, event)
        })
        cy.on("mouseout", "node", (event) => { event.target.removeClass("hovered"); setTooltip(null) })
        cy.on("mouseover", "edge", (event) => { event.target.addClass("hovered"); const relationship = relationships.get(event.target.id()); if (relationship !== undefined) showTooltip({ kind: "relationship", value: relationship }, event) })
        cy.on("mouseout", "edge", (event) => { event.target.removeClass("hovered"); setTooltip(null) })
        cy.on("select", "node, edge", (event) => event.target.addClass("ui-selected"))
        cy.on("unselect", "node, edge", (event) => event.target.removeClass("ui-selected"))
        cy.on("tap", "node", (event) => onEntityRef.current(event.target.id()))
        cy.on("tap", "edge", (event) => onRelationshipRef.current(event.target.id()))
        instance = cy
        cytoscapeRef.current = cy
        runLayout(cy, projection, mode, false)
      }).catch(failInitialization)
    }

    const updateViewport = (): void => {
      const width = container.clientWidth
      const height = container.clientHeight
      const action = graphViewportAction(width, height, instance !== null || initializing)
      if (action === "initialize") initialize()
      if (action === "resize" && instance !== null) {
        instance.resize()
      }
    }
    const observer = new ResizeObserver(updateViewport)
    observer.observe(container)
    updateViewport()
    return () => {
      disposed = true
      observer.disconnect()
      if (cytoscapeRef.current === instance) cytoscapeRef.current = null
      instance?.destroy()
    }
  }, [elements, entities, mode, projection, relationships])

  const missingCount = projection.missing_entity_ids.length
    + projection.missing_relationship_ids.length
    + projection.unresolved_relationship_ids.length
  const seedCount = projection.seed_entity_ids.length + projection.seed_relationship_ids.length
  const missingIds = [
    ...projection.missing_entity_ids,
    ...projection.missing_relationship_ids,
    ...projection.unresolved_relationship_ids,
  ]
  const focused = mode !== "overview"
  const title = mode === "query-focus" ? "Query focus" : mode === "explorer-focus" ? "Focused subgraph" : "Overview"
  const seedLabel = mode === "query-focus" ? "Query seed" : "Seed"

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border bg-card/50 px-2.5 py-2">
        <div className="min-w-0 text-[11px]">
          <div className="flex items-center gap-2 font-medium">
            <span>{title}</span>
            {props.loading ? <LoaderCircle className="size-3.5 animate-spin text-primary" aria-label="Loading graph data" /> : null}
          </div>
          <p className="text-muted-foreground">
            {focused ? `${seedCount} seeds · ` : ""}{projection.entities.length} nodes · {projection.relationships.length} edges
            {mode === "overview" && projection.unresolved_relationship_count > 0 ? ` · ${projection.unresolved_relationship_count} unresolved relationships in source graph` : ""}
            {projection.truncated ? " · bounded view" : ""}
          </p>
          {props.summary !== null ? <p className="text-muted-foreground">{props.summary.entity_count.toLocaleString()} entities · {props.summary.relationship_count.toLocaleString()} relationships · {props.summary.community_count.toLocaleString()} communities · {props.summary.community_report_count.toLocaleString()} reports</p> : null}
        </div>
        <div className="flex items-center gap-1">
          {focused ? <Button variant="outline" size="sm" onClick={props.onBackOverview}>Back to overview</Button> : null}
          <Button variant="ghost" size="icon" title="Fit the current projection" aria-label="Fit graph projection" onClick={() => { const cy = cytoscapeRef.current; if (cy !== null) focusCollection(cy, projection, mode) }}><Focus /></Button>
          <Button variant="ghost" size="icon" title="Re-run layout without loading data" aria-label="Re-layout graph projection" onClick={() => { const cy = cytoscapeRef.current; if (cy !== null) runLayout(cy, projection, mode, true) }}><RefreshCcw /></Button>
          <Button variant="ghost" size="icon" title="Reload graph data from the backend" aria-label="Reload graph data" onClick={props.onReload}><RotateCw /></Button>
        </div>
      </div>
      {props.error !== null ? <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-red-300">{props.error}</p> : null}
      {props.summary === null && props.summaryError ? <p className="rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-xs text-warning">Graph summary is unavailable. The bounded visualization is still available.</p> : null}
      {missingCount > 0 ? (
        <details className="rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-xs">
          <summary className="cursor-pointer text-warning">{missingCount} selected records could not be represented in the graph.</summary>
          <div className="mt-2 flex flex-wrap gap-1 font-mono text-[10px] text-muted-foreground">{missingIds.map((id) => <span key={id} className="rounded bg-muted px-1.5 py-0.5">{id}</span>)}</div>
        </details>
      ) : null}
      <div className="flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground" aria-label="Graph legend">
        <Legend color="var(--graph-person)" label="PERSON" /><Legend color="var(--graph-organization)" label="ORGANIZATION" /><Legend color="var(--graph-geo)" label="GEO" /><Legend color="var(--graph-event)" label="EVENT" /><Legend color="var(--graph-default)" label="Other" />
        <span className="border-l pl-3"><span className="mr-1 inline-block size-2.5 rounded-full border-2 border-[var(--graph-seed)]" />{seedLabel}</span><span><span className="mr-1 inline-block size-2.5 rounded-full bg-[var(--graph-default)]" />Neighbor</span>
      </div>
      <div className="relative min-h-80 flex-1 overflow-hidden rounded-md border bg-background">
        <div className="absolute inset-0">
          <div ref={containerRef} className="size-full" aria-label="Knowledge graph network preview" />
        </div>
        {rendererError ? <div className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-background/95 p-6 text-center" role="alert"><p className="text-sm font-medium text-destructive">Graph visualization failed to initialize.</p><p className="mt-1 text-xs text-muted-foreground">Check the browser console for renderer diagnostics.</p></div> : null}
        {tooltip !== null ? <GraphTooltip {...tooltip} /> : null}
      </div>
    </div>
  )
}

function Legend({ color, label }: { color: string; label: string }): React.ReactElement {
  return <span><span className="mr-1 inline-block size-2.5 rounded-full" style={{ backgroundColor: color }} />{label}</span>
}
