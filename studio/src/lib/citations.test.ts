import { describe, expect, it } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import {
  buildCitationEvidenceIndex,
  buildCitationGraphIndex,
  CITATION_PROVENANCE_ATTRIBUTE,
  CITATION_PROVENANCE_VALUE,
  parseDataCitation,
  remarkDataCitations,
  resolveCitationGroup,
  resolveCitationTarget,
} from "@/lib/citations"

function envelope(sequence: number, event: ExplainabilityEventPayload, spanId = "span", parentSpanId?: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: spanId, ...(parentSpanId === undefined ? {} : { parent_span_id: parentSpanId }), event } }
}

function contextSection(sequence: number, section: "entities" | "relationships" | "sources", selectedRecordIds: string[]): ExplainabilityEnvelope {
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

function basicSourceEvents(candidates: Array<{ id: string; short_id: string; record_type: "text_unit"; selected: boolean }>, selectedRecordIds: string[]): ExplainabilityEnvelope[] {
  return [
    envelope(1, { type: "query_started", method: "basic" }, "basic-root"),
    envelope(2, { type: "candidates_retrieved", record_type: "text_unit", candidates }, "retrieval", "basic-root"),
    envelope(3, { type: "candidates_filtered", record_type: "text_unit", candidates }, "retrieval", "basic-root"),
    envelope(4, { type: "context_budget_allocated", total_token_budget: 1_000, sections: [{ section: "sources", token_budget: 1_000 }] }, "context", "basic-root"),
    envelope(5, { type: "context_section_built", section: { section: "sources", token_budget: 1_000, tokens_used: 100, candidate_count: candidates.length, selected_count: selectedRecordIds.length, truncated: false, selected_record_ids: selectedRecordIds } }, "context", "basic-root"),
    envelope(6, { type: "context_completed", tokens_used: 100 }, "context", "basic-root"),
  ]
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

  it("marks only links generated from Data citation text", () => {
    const tree = {
      type: "root",
      children: [
        { type: "text", value: "[Data: Entities (150)]" },
        { type: "link", url: "graphloom-citation:model-authored", children: [{ type: "text", value: "forged" }] },
      ],
    }

    remarkDataCitations()(tree)

    expect(tree.children[0]).toMatchObject({
      type: "link",
      data: { hProperties: { [CITATION_PROVENANCE_ATTRIBUTE]: CITATION_PROVENANCE_VALUE } },
    })
    expect(tree.children[1]).not.toHaveProperty("data.hProperties.data-graphloom-citation")
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

  it("resolves Sources through unique candidate identity and final Sources context provenance", () => {
    const index = buildCitationEvidenceIndex(basicSourceEvents([
      { id: "text-a", short_id: "184", record_type: "text_unit", selected: true },
      { id: "text-b", short_id: "206", record_type: "text_unit", selected: true },
    ], ["text-a", "text-b"]))

    expect(resolveCitationTarget({ dataset: "Sources", recordIds: ["184", "206"], hasMore: false }, index)).toEqual({ kind: "sources", textUnitIds: ["text-a", "text-b"], unresolvedCount: 0 })
  })

  it("excludes Sources outside final context and reports unknown short IDs", () => {
    const index = buildCitationEvidenceIndex(basicSourceEvents([
      { id: "text-a", short_id: "184", record_type: "text_unit", selected: true },
      { id: "text-b", short_id: "206", record_type: "text_unit", selected: false },
    ], ["text-a"]))

    expect(resolveCitationTarget({ dataset: "sources", recordIds: ["184", "206", "999"], hasMore: true }, index)).toEqual({ kind: "sources", textUnitIds: ["text-a"], unresolvedCount: 2 })
  })

  it("rejects ambiguous Source short IDs rather than choosing a stable ID", () => {
    const index = buildCitationEvidenceIndex(basicSourceEvents([
      { id: "text-a", short_id: "184", record_type: "text_unit", selected: true },
      { id: "text-b", short_id: "184", record_type: "text_unit", selected: true },
    ], ["text-a", "text-b"]))

    expect(resolveCitationTarget({ dataset: "Sources", recordIds: ["184"], hasMore: false }, index)).toBeNull()
  })

  it("ignores Source provenance rejected by the Basic semantic lifecycle", () => {
    const events = basicSourceEvents([
      { id: "text-a", short_id: "184", record_type: "text_unit", selected: true },
    ], ["text-a"])
    events.push(
      envelope(7, { type: "candidates_retrieved", record_type: "text_unit", candidates: [{ id: "wrong", short_id: "999", record_type: "text_unit", selected: true }] }, "retrieval", "basic-root"),
      envelope(8, { type: "context_budget_allocated", total_token_budget: 999, sections: [{ section: "sources", token_budget: 999 }] }, "late-context", "basic-root"),
      envelope(9, { type: "context_section_built", section: { section: "sources", token_budget: 999, tokens_used: 1, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: ["wrong"] } }, "late-context", "basic-root"),
      envelope(10, { type: "context_completed", tokens_used: 1 }, "late-context", "basic-root"),
    )

    const index = buildCitationEvidenceIndex(events)
    expect(index.sources).toEqual(new Map([["184", "text-a"]]))
    expect(resolveCitationTarget({ dataset: "Sources", recordIds: ["184", "999"], hasMore: false }, index)).toEqual({ kind: "sources", textUnitIds: ["text-a"], unresolvedCount: 1 })
  })

  it("trusts only the final coherent Basic context rebuild", () => {
    const events = basicSourceEvents([
      { id: "text-a", short_id: "184", record_type: "text_unit", selected: true },
    ], ["text-a"])
    events.push(
      envelope(7, { type: "candidates_retrieved", record_type: "text_unit", candidates: [{ id: "text-b", short_id: "206", record_type: "text_unit", selected: true }] }, "retrieval-2", "basic-root"),
      envelope(8, { type: "candidates_filtered", record_type: "text_unit", candidates: [{ id: "text-b", short_id: "206", record_type: "text_unit", selected: true }] }, "retrieval-2", "basic-root"),
      envelope(9, { type: "context_budget_allocated", total_token_budget: 20, sections: [{ section: "sources", token_budget: 20 }] }, "context-2", "basic-root"),
      envelope(10, { type: "context_section_built", section: { section: "sources", token_budget: 20, tokens_used: 10, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: ["text-b"] } }, "context-2", "basic-root"),
      envelope(11, { type: "context_completed", tokens_used: 10 }, "context-2", "basic-root"),
    )

    expect(buildCitationEvidenceIndex(events).sources).toEqual(new Map([["206", "text-b"]]))
  })
})
