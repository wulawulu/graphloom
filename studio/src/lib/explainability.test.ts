import { describe, expect, it } from "vitest"

import { deriveFinalGraphFocus, describeEvent, eventSummary, highlightFromEvent, mergeEnvelopes } from "@/lib/explainability"
import type { ExplainabilityEnvelope } from "@/api/types"

const frame = (sequence: number): ExplainabilityEnvelope => ({ schema_version: 1, sequence, record: { run_id: "run", timestamp: "2026-08-09T00:00:00Z", span_id: "span", event: { type: "warning" } } })
const eventFrame = (sequence: number, event: ExplainabilityEnvelope["record"]["event"]): ExplainabilityEnvelope => ({ schema_version: 1, sequence, record: { run_id: "run", timestamp: "2026-08-09T00:00:00Z", span_id: "span", event } })

describe("Explainability presentation helpers", () => {
  it("derives automatic graph focus only from records in the final context", () => {
    const envelopes = [
      eventFrame(4, { type: "relationships_selected", relationships: [{ id: "r-2", selected: true }, { id: "r-rejected", selected: false }, { id: "r-1", selected: true }] }),
      eventFrame(2, { type: "graph_expansion_started", seed_entity_ids: ["e-2", "e-1"] }),
      eventFrame(3, { type: "entities_selected", entities: [{ id: "e-1", selected: true }, { id: "e-rejected", selected: false }] }),
      eventFrame(5, { type: "relationships_selected", relationships: [{ id: "r-1", selected: true }] }),
      eventFrame(7, { type: "context_section_built", section: { section: "relationships", name: "Relationships", token_budget: 20, tokens_used: 5, candidate_count: 2, selected_count: 1, truncated: true, selected_record_ids: ["r-1"] } }),
      eventFrame(6, { type: "context_section_built", section: { section: "entities", name: "Entities", token_budget: 20, tokens_used: 5, candidate_count: 3, selected_count: 1, truncated: true, selected_record_ids: ["e-1"] } }),
    ]

    expect(deriveFinalGraphFocus(envelopes)).toEqual({
      entityIds: ["e-1"],
      relationshipIds: ["r-1"],
    })
  })

  it("returns no final focus without final-context graph evidence", () => {
    expect(deriveFinalGraphFocus([
      eventFrame(1, { type: "entities_selected", entities: [{ id: "e-rejected", selected: false }] }),
      eventFrame(2, { type: "run_completed" }),
    ])).toBeNull()
  })

  it("uses the latest version of a progressively rebuilt context section", () => {
    const section = (selected_record_ids: string[]) => ({
      section: "entities", name: "Entities", token_budget: 20, tokens_used: 5,
      candidate_count: 2, selected_count: selected_record_ids.length, truncated: true, selected_record_ids,
    })
    expect(deriveFinalGraphFocus([
      eventFrame(3, { type: "context_section_built", section: section(["entity-final"]) }),
      eventFrame(1, { type: "context_section_built", section: section(["entity-early"]) }),
    ])).toEqual({ entityIds: ["entity-final"], relationshipIds: [] })
  })

  it("orders and deduplicates frames", () => {
    expect(mergeEnvelopes([frame(3), frame(1)], frame(2)).map((value) => value.sequence)).toEqual([1, 2, 3])
    expect(mergeEnvelopes([frame(1)], frame(1))).toHaveLength(1)
  })

  it("renders future events through the generic fallback", () => {
    expect(describeEvent({ type: "future_graphloom_event", foo: "bar" })).toEqual({ label: "future graphloom event", category: "Lifecycle" })
  })

  it("extracts graph highlights only from supported graph events", () => {
    expect(highlightFromEvent({ type: "entities_selected", entities: [{ id: "e1", selected: true }, { id: "e2", selected: false }] })?.entityIds).toEqual(["e1"])
    expect(highlightFromEvent({ type: "relationships_selected", relationships: [{ id: "r1", selected: true }, { id: "r2", selected: false }] })?.relationshipIds).toEqual(["r1"])
    expect(highlightFromEvent({ type: "graph_expansion_started", seed_entity_ids: ["e3", 4] })?.entityIds).toEqual(["e3"])
    expect(highlightFromEvent({ type: "entities_selected", entities: [{ id: "e1", selected: false }] })).toBeNull()
    expect(highlightFromEvent({ type: "relationships_selected", relationships: [] })).toBeNull()
    expect(highlightFromEvent({ type: "graph_expansion_started", seed_entity_ids: [] })).toBeNull()
    expect(highlightFromEvent({ type: "future_event" })).toBeNull()
  })

  it("summarizes the real LLM token fields", () => {
    expect(eventSummary({ type: "llm_request_completed", model_id: "fixture", input_tokens: 17, output_tokens: 5 }))
      .toContain("input tokens: 17 · output tokens: 5")
  })
})
