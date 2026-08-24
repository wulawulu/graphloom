import { describe, expect, it } from "vitest"

import type { GraphProjection } from "@/api/types"
import { deriveGraphFocusHierarchy } from "@/lib/graph-focus"

describe("graph focus hierarchy", () => {
  it("separates final-context records from structural and neighbor edges", () => {
    const projection: GraphProjection = {
      entities: ["core-a", "core-b", "neighbor-a", "neighbor-b"].map((id) => ({ id, title: id, entity_type: null, degree: 1, rank: 1 })),
      relationships: [
        { id: "context-edge", source_entity_id: "core-a", target_entity_id: "neighbor-a", source: "A", target: "N", weight: 1, rank: 1 },
        { id: "core-connection", source_entity_id: "core-a", target_entity_id: "core-b", source: "A", target: "B", weight: 1, rank: 1 },
        { id: "boundary", source_entity_id: "core-b", target_entity_id: "neighbor-b", source: "B", target: "N", weight: 1, rank: 1 },
        { id: "neighbor-edge", source_entity_id: "neighbor-a", target_entity_id: "neighbor-b", source: "N", target: "N", weight: 1, rank: 1 },
      ],
      seed_entity_ids: [], seed_relationship_ids: [], missing_entity_ids: [], missing_relationship_ids: [],
      unresolved_relationship_ids: [], unresolved_relationship_count: 0, truncated: false,
    }

    expect(deriveGraphFocusHierarchy(projection, { entityIds: ["core-a", "core-b"], relationshipIds: ["context-edge"] })).toEqual({
      coreEntityIds: ["core-a", "core-b"],
      relationshipEndpointEntityIds: ["neighbor-a"],
      neighborEntityIds: ["neighbor-b"],
      coreRelationshipIds: ["context-edge"],
      coreConnectionIds: ["core-connection"],
      boundaryRelationshipIds: ["boundary"],
      neighborRelationshipIds: ["neighbor-edge"],
    })
  })

  it("promotes relationship-only endpoints without changing core evidence", () => {
    const projection = focusedProjection([
      relationship("r", "a", "b"),
      relationship("other", "b", "c"),
    ], ["a", "b", "c", "unrelated"])

    expect(deriveGraphFocusHierarchy(projection, { entityIds: [], relationshipIds: ["r"] })).toEqual({
      coreEntityIds: [],
      relationshipEndpointEntityIds: ["a", "b"],
      neighborEntityIds: ["c", "unrelated"],
      coreRelationshipIds: ["r"],
      coreConnectionIds: [],
      boundaryRelationshipIds: [],
      neighborRelationshipIds: ["other"],
    })
  })

  it("prioritizes core entities over relationship endpoints", () => {
    const projection = focusedProjection([relationship("r", "a", "b")], ["a", "b"])
    const hierarchy = deriveGraphFocusHierarchy(projection, { entityIds: ["a"], relationshipIds: ["r"] })
    expect(hierarchy.coreEntityIds).toEqual(["a"])
    expect(hierarchy.relationshipEndpointEntityIds).toEqual(["b"])
    expect(hierarchy.neighborEntityIds).toEqual([])
  })

  it("deduplicates shared and self-loop relationship endpoints", () => {
    const projection = focusedProjection([
      relationship("r1", "a", "b"),
      relationship("r2", "b", "c"),
      relationship("self", "c", "c"),
    ], ["a", "b", "c"])
    const hierarchy = deriveGraphFocusHierarchy(projection, { entityIds: [], relationshipIds: ["r1", "r2", "self"] })
    expect(hierarchy.relationshipEndpointEntityIds).toEqual(["a", "b", "c"])
  })

  it("keeps full projections unchanged when no focus hierarchy is applied", () => {
    const projection = focusedProjection([relationship("r", "a", "b")], ["a", "b"])
    expect(projection.entities.map((entity) => entity.id)).toEqual(["a", "b"])
    expect(projection.relationships.map((edge) => edge.id)).toEqual(["r"])
  })
})

function relationship(id: string, source: string, target: string): GraphProjection["relationships"][number] {
  return { id, source_entity_id: source, target_entity_id: target, source, target, weight: 1, rank: 1 }
}

function focusedProjection(relationships: GraphProjection["relationships"], entityIds: string[]): GraphProjection {
  return {
    entities: entityIds.map((id) => ({ id, title: id, entity_type: null, degree: 1, rank: 1 })),
    relationships,
    seed_entity_ids: [],
    seed_relationship_ids: [],
    missing_entity_ids: [],
    missing_relationship_ids: [],
    unresolved_relationship_ids: [],
    unresolved_relationship_count: 0,
    truncated: false,
  }
}
