import { describe, expect, it } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { buildCitationGraphIndex, parseDataCitation, resolveCitationGroup } from "@/lib/citations"

function envelope(sequence: number, event: ExplainabilityEventPayload): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event } }
}

describe("GraphRAG data citations", () => {
  it("parses a single Entities citation", () => {
    expect(parseDataCitation("[Data: Entities (150, 0, 119, 130)]")).toEqual({
      raw: "[Data: Entities (150, 0, 119, 130)]",
      groups: [{ dataset: "Entities", recordIds: ["150", "0", "119", "130"], hasMore: false }],
    })
  })

  it("parses mixed group separators, whitespace, and +more", () => {
    expect(parseDataCitation("[Data: Sources (15, 16), Reports (1), Entities (5, 7); Relationships (23); Claims (2, 7, 34, 46, 64, +more)]")?.groups).toEqual([
      { dataset: "Sources", recordIds: ["15", "16"], hasMore: false },
      { dataset: "Reports", recordIds: ["1"], hasMore: false },
      { dataset: "Entities", recordIds: ["5", "7"], hasMore: false },
      { dataset: "Relationships", recordIds: ["23"], hasMore: false },
      { dataset: "Claims", recordIds: ["2", "7", "34", "46", "64"], hasMore: true },
    ])
    expect(parseDataCitation("[Data:   Entities ( 150 , 0 ) ; Relationships ( 23 ) ]")?.groups).toHaveLength(2)
  })

  it("rejects malformed citations and ordinary Markdown brackets", () => {
    expect(parseDataCitation("[Data: Entities 150, 0]")).toBeNull()
    expect(parseDataCitation("[Data: Entities ()]")).toBeNull()
    expect(parseDataCitation("[ordinary link text]")).toBeNull()
  })

  it("maps only selected unique short IDs to stable graph IDs", () => {
    const index = buildCitationGraphIndex([
      envelope(1, { type: "entities_selected", entities: [
        { id: "entity-uuid", short_id: "150", record_type: "entity", selected: true },
        { id: "entity-rejected", short_id: "151", record_type: "entity", selected: false },
        { id: "entity-conflict-a", short_id: "152", record_type: "entity", selected: true },
      ] }),
      envelope(2, { type: "entities_selected", entities: [{ id: "entity-conflict-b", short_id: "152", record_type: "entity", selected: true }] }),
      envelope(3, { type: "relationships_selected", relationships: [{ id: "relationship-uuid", short_id: "23", record_type: "relationship", selected: true }] }),
    ])

    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["150", "151", "152", "unknown"], hasMore: false }, index)).toEqual({ entityIds: ["entity-uuid"], relationshipIds: [] })
    expect(resolveCitationGroup({ dataset: "Relationships", recordIds: ["23"], hasMore: false }, index)).toEqual({ entityIds: [], relationshipIds: ["relationship-uuid"] })
    expect(resolveCitationGroup({ dataset: "Reports", recordIds: ["1"], hasMore: false }, index)).toBeNull()
  })
})
