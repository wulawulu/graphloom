import type { ElementDefinition } from "cytoscape"

import type { GraphProjection, GraphProjectionEntity } from "@/api/types"

export const PERMANENT_GRAPH_LABEL_LIMIT = 24
export const MIN_GRAPH_NODE_WIDTH = 32
export const MAX_GRAPH_NODE_WIDTH = 112

export function graphNodeDisplayWidth(label: string): number {
  return Math.min(MAX_GRAPH_NODE_WIDTH, Math.max(MIN_GRAPH_NODE_WIDTH, 16 + Array.from(label).length * 6))
}

export function permanentEntityLabelIds(
  projection: GraphProjection,
  limit = PERMANENT_GRAPH_LABEL_LIMIT,
): Set<string> {
  const seeds = new Set(projection.seed_entity_ids)
  const ranked = [...projection.entities]
    .filter((entity) => !seeds.has(entity.id))
    .sort(compareEntityLabelPriority)
    .slice(0, limit)
  return new Set([...seeds, ...ranked.map((entity) => entity.id)])
}

function compareEntityLabelPriority(left: GraphProjectionEntity, right: GraphProjectionEntity): number {
  if (left.rank === null && right.rank !== null) return 1
  if (left.rank !== null && right.rank === null) return -1
  if (left.rank !== null && right.rank !== null && left.rank !== right.rank) return right.rank - left.rank
  return left.id < right.id ? -1 : Number(left.id > right.id)
}

export function buildProjectionElements(projection: GraphProjection): ElementDefinition[] {
  const seedEntityIds = new Set(projection.seed_entity_ids)
  const seedRelationshipIds = new Set(projection.seed_relationship_ids)
  const labeledEntityIds = permanentEntityLabelIds(projection)
  const nodes: ElementDefinition[] = projection.entities.map((entity) => ({
    group: "nodes",
    classes: [seedEntityIds.has(entity.id) ? "seed" : "neighbor", labeledEntityIds.has(entity.id) ? "permanent-label" : ""].filter(Boolean).join(" "),
    data: { id: entity.id, label: entity.title, displayWidth: graphNodeDisplayWidth(entity.title), entityType: (entity.entity_type ?? "OTHER").toUpperCase(), rank: entity.rank },
  }))
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

export function graphViewportAction(width: number, initialized: boolean): GraphViewportAction {
  if (width <= 0) return "none"
  return initialized ? "resize" : "initialize"
}
