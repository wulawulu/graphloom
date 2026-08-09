import type { ElementDefinition } from "cytoscape"

import type { GraphProjection } from "@/api/types"

export function buildProjectionElements(projection: GraphProjection): ElementDefinition[] {
  const seedEntityIds = new Set(projection.seed_entity_ids)
  const seedRelationshipIds = new Set(projection.seed_relationship_ids)
  const nodes: ElementDefinition[] = projection.entities.map((entity) => ({
    group: "nodes",
    classes: seedEntityIds.has(entity.id) ? "seed" : "neighbor",
    data: { id: entity.id, label: entity.title, entityType: (entity.entity_type ?? "OTHER").toUpperCase() },
  }))
  const edges: ElementDefinition[] = projection.relationships.map((relationship) => ({
      group: "edges",
      classes: seedRelationshipIds.has(relationship.id) ? "seed-relationship" : undefined,
      data: {
        id: relationship.id,
        source: relationship.source_entity_id,
        target: relationship.target_entity_id,
      },
    }))
  return [...nodes, ...edges]
}

export function projectionFocusIds(projection: GraphProjection, focusMode: boolean): string[] {
  if (focusMode && projection.seed_entity_ids.length > 0) return [...projection.seed_entity_ids]
  return projection.entities.map((entity) => entity.id)
}
