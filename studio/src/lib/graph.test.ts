import { describe, expect, it } from "vitest"

import type { GraphProjection } from "@/api/types"
import {
  MAX_GRAPH_NODE_HEIGHT,
  MAX_GRAPH_NODE_WIDTH,
  MIN_GRAPH_NODE_HEIGHT,
  buildProjectionElements,
  graphNodeDimensions,
  graphViewportAction,
  projectionFocusIds,
} from "@/lib/graph"

const projection: GraphProjection = {
  entities: [
    { id: "entity-seed", title: "Alice", entity_type: "PERSON", degree: 9, rank: 9 },
    { id: "entity-neighbor", title: "Acme", entity_type: "ORGANIZATION", degree: 4, rank: 4 },
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
  it("uses a stable monotonic logarithmic degree scale with clamps", () => {
    const dimensions = (degree: number | null) => graphNodeDimensions({ title: "Hub", degree })

    expect(dimensions(null).height).toBe(MIN_GRAPH_NODE_HEIGHT)
    expect(dimensions(0).height).toBe(MIN_GRAPH_NODE_HEIGHT)
    expect(dimensions(1).height).toBeGreaterThanOrEqual(MIN_GRAPH_NODE_HEIGHT)
    expect(dimensions(10).height).toBeGreaterThan(dimensions(1).height)
    expect(dimensions(100).height).toBeGreaterThan(dimensions(10).height)
    expect(dimensions(Number.MAX_SAFE_INTEGER).height).toBe(MAX_GRAPH_NODE_HEIGHT)
    expect(dimensions(Number.MAX_SAFE_INTEGER).width).toBeLessThanOrEqual(MAX_GRAPH_NODE_WIDTH)
  })

  it("uses code-point-aware bounded dimensions for Unicode labels", () => {
    const short = graphNodeDimensions({ title: "甲", degree: 0 })
    const medium = graphNodeDimensions({ title: "西门庆与潘金莲", degree: 0 })
    const long = graphNodeDimensions({ title: "非常长的中文实体名称".repeat(20), degree: 100 })

    expect(medium.width).toBeGreaterThan(short.width)
    expect(long.width).toBe(MAX_GRAPH_NODE_WIDTH)
    expect(long.height).toBe(MAX_GRAPH_NODE_HEIGHT)
  })

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

  it("shows every entity title by default with final dimensions before layout", () => {
    const nodes = buildProjectionElements(projection).filter((element) => element.group === "nodes")

    expect(nodes).toHaveLength(projection.entities.length)
    for (const entity of projection.entities) {
      const node = nodes.find((element) => element.data.id === entity.id)
      expect(node?.data.label).toBe(entity.title)
      expect(node?.data.displayWidth).toEqual(expect.any(Number))
      expect(node?.data.displayHeight).toEqual(expect.any(Number))
      expect(node?.classes).not.toContain("permanent-label")
    }
  })

  it("keeps identical entity dimensions across different projections", () => {
    const entity = { id: "same", title: "Same entity", entity_type: "PERSON", degree: 30, rank: 30 }
    const seed = projection.entities.at(0)
    if (seed === undefined) throw new Error("projection fixture must contain a seed")
    const first = buildProjectionElements({ ...projection, entities: [entity] })[0]
    const second = buildProjectionElements({ ...projection, entities: [seed, entity] })[1]

    expect(first?.data.displayWidth).toBe(second?.data.displayWidth)
    expect(first?.data.displayHeight).toBe(second?.data.displayHeight)
  })

  it("focuses seed nodes in focus mode and all nodes in overview mode", () => {
    expect(projectionFocusIds(projection, true)).toEqual(["entity-seed"])
    expect(projectionFocusIds(projection, false)).toEqual(["entity-seed", "entity-neighbor"])
  })

  it("initializes only when both viewport dimensions are positive", () => {
    expect(graphViewportAction(1_396, 0, false)).toBe("none")
    expect(graphViewportAction(0, 474, false)).toBe("none")
    expect(graphViewportAction(1_396, 474, false)).toBe("initialize")
    expect(graphViewportAction(1_396, 474, true)).toBe("resize")
  })

})
