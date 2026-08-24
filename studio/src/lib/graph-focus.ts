import type { GraphProjection } from "@/api/types"
import type { GraphHighlight } from "@/lib/explainability"

export interface GraphFocusHierarchy {
  coreEntityIds: string[]
  relationshipEndpointEntityIds: string[]
  neighborEntityIds: string[]
  coreRelationshipIds: string[]
  coreConnectionIds: string[]
  boundaryRelationshipIds: string[]
  neighborRelationshipIds: string[]
}

export function deriveGraphFocusHierarchy(
  projection: GraphProjection,
  core: GraphHighlight,
): GraphFocusHierarchy {
  const coreEntities = new Set(core.entityIds)
  const coreRelationships = new Set(core.relationshipIds)
  const relationshipEndpoints = new Set<string>()
  for (const relationship of projection.relationships) {
    if (!coreRelationships.has(relationship.id)) continue
    if (!coreEntities.has(relationship.source_entity_id)) relationshipEndpoints.add(relationship.source_entity_id)
    if (!coreEntities.has(relationship.target_entity_id)) relationshipEndpoints.add(relationship.target_entity_id)
  }
  const hierarchy: GraphFocusHierarchy = {
    coreEntityIds: [],
    relationshipEndpointEntityIds: [],
    neighborEntityIds: [],
    coreRelationshipIds: [],
    coreConnectionIds: [],
    boundaryRelationshipIds: [],
    neighborRelationshipIds: [],
  }

  for (const entity of projection.entities) {
    if (coreEntities.has(entity.id)) hierarchy.coreEntityIds.push(entity.id)
    else if (relationshipEndpoints.has(entity.id)) hierarchy.relationshipEndpointEntityIds.push(entity.id)
    else hierarchy.neighborEntityIds.push(entity.id)
  }
  for (const relationship of projection.relationships) {
    if (coreRelationships.has(relationship.id)) {
      hierarchy.coreRelationshipIds.push(relationship.id)
      continue
    }
    const coreEndpointCount = Number(coreEntities.has(relationship.source_entity_id))
      + Number(coreEntities.has(relationship.target_entity_id))
    if (coreEndpointCount === 2) hierarchy.coreConnectionIds.push(relationship.id)
    else if (coreEndpointCount === 1) hierarchy.boundaryRelationshipIds.push(relationship.id)
    else hierarchy.neighborRelationshipIds.push(relationship.id)
  }
  return hierarchy
}
