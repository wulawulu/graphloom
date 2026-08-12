import { describe, expect, it } from "vitest"

import type { GraphProjection } from "@/api/types"
import { buildProjectionElements, graphViewportAction, permanentEntityLabelIds, projectionFocusIds } from "@/lib/graph"

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
    expect(elements.find((element) => element.data.id === "entity-seed")?.classes).toContain("seed")
    expect(elements.find((element) => element.data.id === "entity-neighbor")?.classes).toContain("neighbor")
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

  it("labels seeds first, then higher ranks with a stable ID tiebreak", () => {
    const labels = permanentEntityLabelIds({
      ...projection,
      entities: [
        { id: "seed", title: "Seed", entity_type: null, rank: null },
        { id: "z-low", title: "Low", entity_type: null, rank: 1 },
        { id: "b-high", title: "B", entity_type: null, rank: 9 },
        { id: "a-high", title: "A", entity_type: null, rank: 9 },
      ],
      seed_entity_ids: ["seed"],
    }, 2)

    expect([...labels]).toEqual(["seed", "a-high", "b-high"])
    expect(labels.has("z-low")).toBe(false)
  })

  it("uses stable ID order when nullable ranks are tied", () => {
    const labels = permanentEntityLabelIds({
      ...projection,
      entities: [
        { id: "z-null", title: "Z", entity_type: null, rank: null },
        { id: "a-null", title: "A", entity_type: null, rank: null },
      ],
      seed_entity_ids: [],
    }, 1)
    expect([...labels]).toEqual(["a-null"])
  })
})
