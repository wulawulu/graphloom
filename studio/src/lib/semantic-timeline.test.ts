import { describe, expect, it } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { buildSemanticTimeline } from "@/lib/semantic-timeline"

function envelope(sequence: number, event: ExplainabilityEventPayload): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run", timestamp: `2026-08-19T00:00:${String(sequence).padStart(2, "0")}.000Z`, span_id: `span-${sequence}`, event } }
}

function section(sequence: number, kind: string, ids: string[]): ExplainabilityEnvelope {
  return envelope(sequence, { type: "context_section_built", section: { section: kind, token_budget: 1_000, tokens_used: 200, candidate_count: 2, selected_count: ids.length, truncated: ids.length < 2, selected_record_ids: ids } })
}

describe("semantic explainability timeline", () => {
  it("aggregates a basic Local Query into four decision steps", () => {
    const model = buildSemanticTimeline([
      envelope(1, { type: "run_started" }),
      envelope(2, { type: "query_started", method: "local" }),
      envelope(3, { type: "embedding_started", model_id: "bge-m3" }),
      envelope(4, { type: "embedding_completed", model_id: "bge-m3", prompt_tokens: 12, dimensions: 1_024 }),
      envelope(5, { type: "candidates_retrieved", candidates: [{ id: "entity-1", record_type: "entity", selected: false }] }),
      envelope(6, { type: "entities_selected", entities: [{ id: "entity-1", record_type: "entity", selected: true }] }),
      envelope(7, { type: "relationships_selected", relationships: [{ id: "relationship-1", record_type: "relationship", selected: true }] }),
      section(8, "entities", ["entity-1"]),
      envelope(9, { type: "context_completed", tokens_used: 200 }),
      envelope(10, { type: "llm_request_started", model_id: "model", prompt_tokens: 200 }),
      envelope(11, { type: "llm_request_completed", model_id: "model", input_tokens: 200, output_tokens: 20, elapsed_ms: 50 }),
      envelope(12, { type: "run_completed", elapsed_ms: 100 }),
    ])

    expect(model.steps.map((step) => step.title)).toEqual(["Entity Mapping", "Graph Expansion", "Context Assembly", "Answer Generation"])
    expect(model.diagnosticEvents.map((item) => item.record.event.type)).toEqual(["run_started", "query_started", "run_completed"])
  })

  it("keeps Entity Mapping when embedding diagnostics are absent", () => {
    const model = buildSemanticTimeline([
      envelope(1, { type: "candidates_retrieved", candidates: [{ id: "entity-1", record_type: "entity", selected: false }] }),
      envelope(2, { type: "entities_selected", entities: [{ id: "entity-1", record_type: "entity", selected: true }] }),
    ])

    expect(model.steps.map((step) => step.kind)).toEqual(["entity-mapping"])
  })

  it.each([
    [[section(3, "entities", ["entity-included"])], "entity-included", "included"],
    [[section(3, "entities", ["entity-other"])], "entity-excluded", "excluded"],
    [[], "entity-unknown", "unknown"],
  ] as const)("derives final-context status from section evidence", (contextEvents, entityId, expected) => {
    const model = buildSemanticTimeline([
      envelope(1, { type: "entities_selected", entities: [{ id: entityId, record_type: "entity", selected: true }] }),
      ...contextEvents,
    ])
    const mapping = model.steps.find((step) => step.kind === "entity-mapping")
    if (mapping?.kind !== "entity-mapping") throw new Error("expected entity mapping")

    expect(mapping.summary.candidates[0]?.selected).toBe(true)
    expect(mapping.summary.candidates[0]?.finalContext).toBe(expected)
  })

  it("merges progressive candidate decisions deterministically and uses the latest section", () => {
    const model = buildSemanticTimeline([
      envelope(4, { type: "entities_selected", entities: [{ id: "entity-1", title: "Alice final", record_type: "entity", rank: 1, selected: true, reason: "ann_result" }] }),
      envelope(1, { type: "candidates_retrieved", candidates: [{ id: "entity-1", title: "Alice", record_type: "entity", rank: 2, selected: false }] }),
      section(2, "entities", ["entity-1"]),
      section(5, "entities", []),
      envelope(3, { type: "candidates_filtered", candidates: [{ id: "entity-1", title: "Alice filtered", record_type: "entity", rank: 1, selected: true }] }),
    ])
    const mapping = model.steps.find((step) => step.kind === "entity-mapping")
    if (mapping?.kind !== "entity-mapping") throw new Error("expected entity mapping")

    expect(mapping.summary.candidates).toHaveLength(1)
    expect(mapping.summary.candidates[0]).toMatchObject({ stableId: "entity-1", title: "Alice final", selected: true, finalContext: "excluded" })
  })

  it("derives relationship final-context membership independently", () => {
    const model = buildSemanticTimeline([
      envelope(1, { type: "relationships_selected", relationships: [{ id: "relationship-in", record_type: "relationship", selected: true }, { id: "relationship-out", record_type: "relationship", selected: true }] }),
      section(2, "relationships", ["relationship-in"]),
    ])
    const expansion = model.steps.find((step) => step.kind === "graph-expansion")
    if (expansion?.kind !== "graph-expansion") throw new Error("expected graph expansion")

    expect(expansion.summary.records).toEqual([
      expect.objectContaining({ stableId: "relationship-in", finalContext: "included" }),
      expect.objectContaining({ stableId: "relationship-out", finalContext: "excluded" }),
    ])
  })
})
