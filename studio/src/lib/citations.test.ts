import { describe, expect, it } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { buildCitationGraphIndex, parseDataCitation, resolveCitationGroup } from "@/lib/citations"

function envelope(sequence: number, event: ExplainabilityEventPayload): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event } }
}

function contextSection(sequence: number, section: "entities" | "relationships", selectedRecordIds: string[]): ExplainabilityEnvelope {
  return envelope(sequence, {
    type: "context_section_built",
    section: {
      section,
      token_budget: 1_000,
      tokens_used: 100,
      candidate_count: selectedRecordIds.length,
      selected_count: selectedRecordIds.length,
      truncated: false,
      selected_record_ids: selectedRecordIds,
    },
  })
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

  it("maps a selected entity only when it entered the final entity context", () => {
    const index = buildCitationGraphIndex([
      envelope(1, { type: "entities_selected", entities: [{ id: "entity-150", short_id: "150", record_type: "entity", selected: true }] }),
      contextSection(2, "entities", ["entity-150"]),
    ])

    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["150"], hasMore: false }, index)).toEqual({ entityIds: ["entity-150"], relationshipIds: [] })
  })

  it("does not map a selected entity removed by context token fitting", () => {
    const index = buildCitationGraphIndex([
      envelope(1, { type: "entities_selected", entities: [
        { id: "entity-150", short_id: "150", record_type: "entity", selected: true },
        { id: "entity-151", short_id: "151", record_type: "entity", selected: true },
      ] }),
      contextSection(2, "entities", ["entity-150"]),
    ])

    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["150"], hasMore: false }, index)).not.toBeNull()
    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["151"], hasMore: false }, index)).toBeNull()
  })

  it("limits relationships to final relationship context membership", () => {
    const index = buildCitationGraphIndex([
      envelope(1, { type: "relationships_selected", relationships: [
        { id: "relationship-23", short_id: "23", record_type: "relationship", selected: true },
        { id: "relationship-24", short_id: "24", record_type: "relationship", selected: true },
      ] }),
      contextSection(2, "relationships", ["relationship-23"]),
    ])

    expect(resolveCitationGroup({ dataset: "Relationships", recordIds: ["23"], hasMore: false }, index)).toEqual({ entityIds: [], relationshipIds: ["relationship-23"] })
    expect(resolveCitationGroup({ dataset: "Relationships", recordIds: ["24"], hasMore: false }, index)).toBeNull()
  })

  it("rejects unselected and conflicting identity mappings", () => {
    const index = buildCitationGraphIndex([
      envelope(1, { type: "entities_selected", entities: [
        { id: "entity-unselected", short_id: "151", record_type: "entity", selected: false },
        { id: "entity-conflict-a", short_id: "152", record_type: "entity", selected: true },
      ] }),
      envelope(2, { type: "entities_selected", entities: [{ id: "entity-conflict-b", short_id: "152", record_type: "entity", selected: true }] }),
      contextSection(3, "entities", ["entity-unselected", "entity-conflict-a", "entity-conflict-b"]),
    ])

    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["151"], hasMore: false }, index)).toBeNull()
    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["152"], hasMore: false }, index)).toBeNull()
  })

  it("is independent of context and selection event order", () => {
    const index = buildCitationGraphIndex([
      contextSection(1, "entities", ["entity-150"]),
      envelope(2, { type: "entities_selected", entities: [{ id: "entity-150", short_id: "150", record_type: "entity", selected: true }] }),
    ])

    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["150"], hasMore: false }, index)).toEqual({ entityIds: ["entity-150"], relationshipIds: [] })
  })

  it("does not infer final context membership when its event is absent", () => {
    const index = buildCitationGraphIndex([
      envelope(1, { type: "entities_selected", entities: [{ id: "entity-150", short_id: "150", record_type: "entity", selected: true }] }),
    ])

    expect(resolveCitationGroup({ dataset: "Entities", recordIds: ["150"], hasMore: false }, index)).toBeNull()
    expect(resolveCitationGroup({ dataset: "Reports", recordIds: ["1"], hasMore: false }, index)).toBeNull()
  })
})
