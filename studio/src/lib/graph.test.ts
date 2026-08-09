import { describe, expect, it } from "vitest"

import type { GraphProjection } from "@/api/types"
import { buildProjectionElements, graphViewportAction, projectionFocusIds } from "@/lib/graph"

const projection: GraphProjection = {
  entities: [
    { id: "entity-seed", title: "Alice", entity_type: "PERSON", rank: 9 },
    { id: "entity-neighbor", title: "Acme", entity_type: "ORGANIZATION", rank: 4 },
  ],
  relationships: [{
    id: "relationship-seed",
    source_entity_id: "entity-neighbor",
    target_entity_id: "entity-seed",
    source: "Acme",
    target: "Alice",
    weight: 0.8,
    rank: 5,
  }],
  seed_entity_ids: ["entity-seed"],
  seed_relationship_ids: ["relationship-seed"],
  missing_entity_ids: [],
  missing_relationship_ids: [],
  unresolved_relationship_ids: [],
  unresolved_relationship_count: 0,
  truncated: false,
}

describe("graph projection transformation", () => {
  it("uses backend-resolved endpoint IDs without title lookup", () => {
    const elements = buildProjectionElements(projection)
    const edge = elements.find((element) => element.data.id === "relationship-seed")
    expect(edge?.data.source).toBe("entity-neighbor")
    expect(edge?.data.target).toBe("entity-seed")
  })

  it("marks entity and relationship seeds with distinct classes", () => {
    const elements = buildProjectionElements(projection)
    expect(elements.find((element) => element.data.id === "entity-seed")?.classes).toBe("seed")
    expect(elements.find((element) => element.data.id === "entity-neighbor")?.classes).toBe("neighbor")
    expect(elements.find((element) => element.data.id === "relationship-seed")?.classes).toBe("seed-relationship")
  })

  it("focuses seed nodes in focus mode and all nodes in overview mode", () => {
    expect(projectionFocusIds(projection, true)).toEqual(["entity-seed"])
    expect(projectionFocusIds(projection, false)).toEqual(["entity-seed", "entity-neighbor"])
  })

  it("initializes when a hidden graph first receives a positive viewport", () => {
    expect(graphViewportAction(0, false)).toBe("none")
    expect(graphViewportAction(640, false)).toBe("initialize")
    expect(graphViewportAction(800, true)).toBe("resize")
  })
})
