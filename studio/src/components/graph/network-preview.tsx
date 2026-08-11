import { useEffect, useMemo, useRef } from "react"
import type { Core } from "cytoscape"
import { Focus, LoaderCircle, RefreshCcw, RotateCw } from "lucide-react"

import type { GraphProjection } from "@/api/types"
import type { GraphViewMode } from "@/components/graph/graph-explorer"
import { Button } from "@/components/ui/button"
import { buildProjectionElements, graphViewportAction, projectionFocusIds } from "@/lib/graph"

interface NetworkPreviewProps {
  projection: GraphProjection
  mode: GraphViewMode
  loading: boolean
  error: string | null
  onEntity: (id: string) => void
  onRelationship: (id: string) => void
  onBackOverview: () => void
  onReload: () => void
}

const layoutOptions = {
  // Cytoscape's compound spring embedder layout name is assembled to keep spellcheck signal clean.
  name: "co" + "se",
  animate: false,
  randomize: true,
  nodeRepulsion: () => 9_000,
  idealEdgeLength: () => 72,
  gravity: 0.22,
  componentSpacing: 72,
  padding: 36,
} as const

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
    cy.fit(targets, 36)
  }
}

function runLayout(cy: Core, projection: GraphProjection, mode: GraphViewMode, animate: boolean): void {
  const options = { ...layoutOptions, animate } as unknown as Parameters<Core["layout"]>[0]
  const layout = cy.layout(options)
  layout.one("layoutstop", () => focusCollection(cy, projection, mode))
  layout.run()
}

function cssVariable(styles: CSSStyleDeclaration, name: string): string {
  return styles.getPropertyValue(name).trim()
}

export function NetworkPreview(props: NetworkPreviewProps): React.ReactElement {
  const { mode, onEntity, onRelationship, projection } = props
  const containerRef = useRef<HTMLDivElement>(null)
  const cytoscapeRef = useRef<Core | null>(null)
  const elements = useMemo(() => buildProjectionElements(projection), [projection])

  useEffect(() => {
    const container = containerRef.current
    if (container === null) return undefined
    let disposed = false
    let initializing = false
    let instance: Core | null = null
    let wasVisible = false

    const initialize = (): void => {
      if (disposed || initializing || instance !== null) return
      initializing = true
      void import("cytoscape").then(({ default: cytoscape }) => {
        if (disposed) return
        const styles = getComputedStyle(document.documentElement)
        const foreground = cssVariable(styles, "--foreground")
        const background = cssVariable(styles, "--background")
        const border = cssVariable(styles, "--border")
        const seed = cssVariable(styles, "--graph-seed")
        const cy = cytoscape({
        container,
        elements,
        layout: { name: "preset" },
        style: [
          { selector: "node", style: { "background-color": cssVariable(styles, "--graph-default"), "border-color": background, "border-width": 2, label: "", width: 21, height: 21 } },
          { selector: 'node[entityType = "PERSON"]', style: { "background-color": cssVariable(styles, "--graph-person") } },
          { selector: 'node[entityType = "ORGANIZATION"], node[entityType = "ORG"]', style: { "background-color": cssVariable(styles, "--graph-organization") } },
          { selector: 'node[entityType = "GEO"]', style: { "background-color": cssVariable(styles, "--graph-geo") } },
          { selector: 'node[entityType = "EVENT"]', style: { "background-color": cssVariable(styles, "--graph-event") } },
          { selector: "node.neighbor", style: { width: 22, height: 22 } },
          { selector: "node.hovered, node.seed, node.ui-selected", style: { label: "data(label)", color: foreground, "font-size": 10, "text-background-color": background, "text-background-opacity": 0.92, "text-background-padding": "3px", "text-background-shape": "roundrectangle", "text-margin-y": -12, "z-index": 10 } },
          { selector: "node.hovered", style: { width: 25, height: 25, "border-width": 3 } },
          { selector: "node.seed", style: { width: 30, height: 30, "border-color": seed, "border-width": 4 } },
          { selector: "node.ui-selected", style: { "border-color": foreground, "border-style": "double", "border-width": 4 } },
          { selector: "edge", style: { width: 1.4, opacity: 0.72, "line-color": border, "target-arrow-color": border, "target-arrow-shape": "triangle", "arrow-scale": 0.75, "curve-style": "bezier" } },
          { selector: "edge.seed-relationship", style: { width: 4, opacity: 1, "line-color": seed, "target-arrow-color": seed, "arrow-scale": 1.15, "z-index": 8 } },
          { selector: "edge.ui-selected", style: { width: 3, opacity: 1, "line-color": foreground, "target-arrow-color": foreground } },
        ],
        minZoom: 0.2,
        maxZoom: 3,
        })
        cy.on("mouseover", "node", (event) => event.target.addClass("hovered"))
        cy.on("mouseout", "node", (event) => event.target.removeClass("hovered"))
        cy.on("select", "node, edge", (event) => event.target.addClass("ui-selected"))
        cy.on("unselect", "node, edge", (event) => event.target.removeClass("ui-selected"))
        cy.on("tap", "node", (event) => onEntity(event.target.id()))
        cy.on("tap", "edge", (event) => onRelationship(event.target.id()))
        instance = cy
        cytoscapeRef.current = cy
        runLayout(cy, projection, mode, false)
      }).catch(() => { initializing = false })
    }

    const updateViewport = (): void => {
      const width = container.clientWidth
      const action = graphViewportAction(width, instance !== null || initializing)
      const becameVisible = width > 0 && !wasVisible
      wasVisible = width > 0
      if (action === "initialize") initialize()
      if (action === "resize" && instance !== null) {
        instance.resize()
        if (becameVisible) focusCollection(instance, projection, mode)
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
  }, [elements, mode, onEntity, onRelationship, projection])

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
        </div>
        <div className="flex items-center gap-1">
          {focused ? <Button variant="outline" size="sm" onClick={props.onBackOverview}>Back to overview</Button> : null}
          <Button variant="ghost" size="icon" title="Fit the current projection" aria-label="Fit graph projection" onClick={() => { const cy = cytoscapeRef.current; if (cy !== null) focusCollection(cy, projection, mode) }}><Focus /></Button>
          <Button variant="ghost" size="icon" title="Re-run layout without loading data" aria-label="Re-layout graph projection" onClick={() => { const cy = cytoscapeRef.current; if (cy !== null) runLayout(cy, projection, mode, true) }}><RefreshCcw /></Button>
          <Button variant="ghost" size="icon" title="Reload graph data from the backend" aria-label="Reload graph data" onClick={props.onReload}><RotateCw /></Button>
        </div>
      </div>
      {props.error !== null ? <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-red-300">{props.error}</p> : null}
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
      <div ref={containerRef} className="min-h-80 flex-1 rounded-md border bg-background" aria-label="Knowledge graph network preview" />
    </div>
  )
}

function Legend({ color, label }: { color: string; label: string }): React.ReactElement {
  return <span><span className="mr-1 inline-block size-2.5 rounded-full" style={{ backgroundColor: color }} />{label}</span>
}
