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
      neighborEntityIds: ["neighbor-a", "neighbor-b"],
      coreRelationshipIds: ["context-edge"],
      coreConnectionIds: ["core-connection"],
      boundaryRelationshipIds: ["boundary"],
      neighborRelationshipIds: ["neighbor-edge"],
    })
  })
})
