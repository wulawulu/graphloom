import { describe, expect, it } from "vitest"

import { describeEvent, eventSummary, highlightFromEvent, mergeEnvelopes } from "@/lib/explainability"
import type { ExplainabilityEnvelope } from "@/api/types"

const frame = (sequence: number): ExplainabilityEnvelope => ({ schema_version: 1, sequence, record: { run_id: "run", timestamp: "2026-08-09T00:00:00Z", span_id: "span", event: { type: "warning" } } })

describe("Explainability presentation helpers", () => {
  it("orders and deduplicates frames", () => {
    expect(mergeEnvelopes([frame(3), frame(1)], frame(2)).map((value) => value.sequence)).toEqual([1, 2, 3])
    expect(mergeEnvelopes([frame(1)], frame(1))).toHaveLength(1)
  })

  it("renders future events through the generic fallback", () => {
    expect(describeEvent({ type: "future_graphloom_event", foo: "bar" })).toEqual({ label: "future graphloom event", category: "Lifecycle" })
  })

  it("extracts graph highlights only from supported graph events", () => {
    expect(highlightFromEvent({ type: "entities_selected", entities: [{ id: "e1" }, { id: "e2" }] })?.entityIds).toEqual(["e1", "e2"])
    expect(highlightFromEvent({ type: "relationships_selected", relationships: [{ id: "r1" }] })?.relationshipIds).toEqual(["r1"])
    expect(highlightFromEvent({ type: "graph_expansion_started", seed_entity_ids: ["e3", 4] })?.entityIds).toEqual(["e3"])
    expect(highlightFromEvent({ type: "future_event" })).toBeNull()
  })

  it("summarizes the real LLM token fields", () => {
    expect(eventSummary({ type: "llm_request_completed", model_id: "fixture", input_tokens: 17, output_tokens: 5 }))
      .toContain("input tokens: 17 · output tokens: 5")
  })
})
