import { describe, expect, it } from "vitest"

import type { GraphEntity, GraphRelationship } from "@/api/types"
import { applyGraphHighlights, buildGraphElements, type HighlightableGraph } from "@/lib/graph"

const entity = (id: string, title: string): GraphEntity => ({ id, title, short_id: null, entity_type: null, rank: null, community_ids: [] })
const relationship = (id: string, source: string, target: string): GraphRelationship => ({ id, source, target, short_id: null, weight: null, rank: null })

describe("graph preview transformation", () => {
  it("resolves only unique loaded title endpoints", () => {
    const result = buildGraphElements(
      [entity("a1", "Alice"), entity("a2", "Alice"), entity("b", "Bob"), entity("c", "Carol")],
      [relationship("ambiguous", "Alice", "Bob"), relationship("missing", "Nobody", "Bob"), relationship("resolved", "Bob", "Carol")],
    )
    expect(result.resolvedRelationshipCount).toBe(1)
    expect(result.omittedRelationshipCount).toBe(2)
    expect(result.elements.some((value) => value.data.id === "resolved")).toBe(true)
    expect(result.elements.some((value) => value.data.id === "ambiguous")).toBe(false)
  })

  it("applies highlights that existed before the graph instance became ready", () => {
    const added: string[] = []
    let removed = false
    const graph: HighlightableGraph = {
      batch: (callback) => callback(),
      elements: () => ({ removeClass: () => { removed = true } }),
      getElementById: (id) => ({ addClass: () => added.push(id) }),
    }
    applyGraphHighlights(graph, new Set(["entity-1"]), new Set(["relationship-2"]))
    expect(removed).toBe(true)
    expect(added).toEqual(["entity-1", "relationship-2"])
  })
})
