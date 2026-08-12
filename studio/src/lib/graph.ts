import type { CoseLayoutOptions, ElementDefinition } from "cytoscape"

import type { GraphProjection, GraphProjectionEntity } from "@/api/types"

export const MIN_GRAPH_NODE_WIDTH = 56
export const MAX_GRAPH_NODE_WIDTH = 136
export const MIN_GRAPH_NODE_HEIGHT = 28
export const MAX_GRAPH_NODE_HEIGHT = 54

export const GRAPH_LAYOUT_OPTIONS = {
  // Cytoscape's compound spring embedder layout name is assembled to keep spellcheck signal clean.
  name: ("co" + "se") as CoseLayoutOptions["name"],
  animate: false,
  randomize: true,
  nodeRepulsion: () => 22_000,
  nodeOverlap: 40,
  idealEdgeLength: () => 130,
  gravity: 0.18,
  componentSpacing: 120,
  padding: 64,
} as const satisfies CoseLayoutOptions

export interface GraphNodeDimensions {
  width: number
  height: number
}

function graphDegreeSizeBonus(degree: number | null): number {
  if (degree === null || !Number.isFinite(degree) || degree <= 0) return 0
  const normalized = Math.min(1, Math.log2(degree + 1) / 6)
  return Math.round((MAX_GRAPH_NODE_HEIGHT - MIN_GRAPH_NODE_HEIGHT) * normalized)
}

export function graphNodeDimensions(entity: Pick<GraphProjectionEntity, "degree" | "title">): GraphNodeDimensions {
  const degreeBonus = graphDegreeSizeBonus(entity.degree)
  const labelWidth = 16 + Array.from(entity.title).length * 6
  return {
    width: Math.min(MAX_GRAPH_NODE_WIDTH, Math.max(MIN_GRAPH_NODE_WIDTH, labelWidth + Math.round(degreeBonus * 0.55))),
    height: MIN_GRAPH_NODE_HEIGHT + degreeBonus,
  }
}

export function buildProjectionElements(projection: GraphProjection): ElementDefinition[] {
  const seedEntityIds = new Set(projection.seed_entity_ids)
  const seedRelationshipIds = new Set(projection.seed_relationship_ids)
  const nodes: ElementDefinition[] = projection.entities.map((entity) => {
    const dimensions = graphNodeDimensions(entity)
    return {
      group: "nodes",
      classes: seedEntityIds.has(entity.id) ? "seed" : "neighbor",
      data: {
        id: entity.id,
        label: entity.title,
        displayWidth: dimensions.width,
        displayHeight: dimensions.height,
        textMaxWidth: `${dimensions.width - 16}px`,
        entityType: (entity.entity_type ?? "OTHER").toUpperCase(),
        degree: entity.degree,
        rank: entity.rank,
      },
    }
  })
  const edges: ElementDefinition[] = projection.relationships.map((relationship) => ({
      group: "edges",
      classes: seedRelationshipIds.has(relationship.id) ? "seed-relationship" : undefined,
      data: {
        id: relationship.id,
        source: relationship.source_entity_id,
        target: relationship.target_entity_id,
        sourceLabel: relationship.source,
        targetLabel: relationship.target,
        weight: relationship.weight,
        rank: relationship.rank,
      },
    }))
  return [...nodes, ...edges]
}

export function projectionFocusIds(projection: GraphProjection, focusMode: boolean): string[] {
  if (focusMode && projection.seed_entity_ids.length > 0) return [...projection.seed_entity_ids]
  return projection.entities.map((entity) => entity.id)
}

export type GraphViewportAction = "none" | "initialize" | "resize"

export function graphViewportAction(width: number, height: number, initialized: boolean): GraphViewportAction {
  if (width <= 0 || height <= 0) return "none"
  return initialized ? "resize" : "initialize"
}
