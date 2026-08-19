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

  it("keeps retrieved-only candidates pending instead of excluding them", () => {
    const model = buildSemanticTimeline([
      envelope(1, { type: "candidates_retrieved", candidates: [{ id: "entity-1", record_type: "entity", selected: false }, { id: "entity-2", record_type: "entity", selected: false }] }),
    ])
    const mapping = model.steps.find((step) => step.kind === "entity-mapping")
    if (mapping?.kind !== "entity-mapping") throw new Error("expected entity mapping")

    expect(mapping.summary.candidates.map((candidate) => candidate.selectionStatus)).toEqual(["pending", "pending"])
    expect(mapping.summary).toMatchObject({ retrievedCount: 2, selectedCount: 0, excludedCount: 0, pendingCount: 2 })
  })

  it("promotes a retrieved candidate to selected when filtering accepts it", () => {
    const model = buildSemanticTimeline([
      envelope(1, { type: "candidates_retrieved", candidates: [{ id: "entity-1", record_type: "entity", selected: false }] }),
      envelope(2, { type: "candidates_filtered", candidates: [{ id: "entity-1", record_type: "entity", selected: true, reason: "ann_result" }] }),
    ])
    const mapping = model.steps.find((step) => step.kind === "entity-mapping")
    if (mapping?.kind !== "entity-mapping") throw new Error("expected entity mapping")

    expect(mapping.summary.candidates[0]).toMatchObject({ selectionStatus: "selected", selected: true, reason: "ann_result" })
  })

  it("promotes a retrieved candidate to excluded when filtering rejects it", () => {
    const model = buildSemanticTimeline([
      envelope(1, { type: "candidates_retrieved", candidates: [{ id: "entity-1", record_type: "entity", selected: false }] }),
      envelope(2, { type: "candidates_filtered", candidates: [{ id: "entity-1", record_type: "entity", selected: false, reason: "explicitly_excluded" }] }),
    ])
    const mapping = model.steps.find((step) => step.kind === "entity-mapping")
    if (mapping?.kind !== "entity-mapping") throw new Error("expected entity mapping")

    expect(mapping.summary.candidates[0]).toMatchObject({ selectionStatus: "excluded", selected: false, reason: "explicitly_excluded" })
    expect(mapping.summary).toMatchObject({ selectedCount: 0, excludedCount: 1, pendingCount: 0 })
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

    expect(mapping.summary.candidates[0]).toMatchObject({ selected: true, selectionStatus: "selected", finalContext: expected })
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
    expect(mapping.summary.candidates[0]).toMatchObject({ stableId: "entity-1", title: "Alice final", selected: true, selectionStatus: "selected", finalContext: "excluded" })
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

  it("uses the latest captured ContextCompleted input without rebuilding it from sections", () => {
    const exact = "Reports\n[]\n\nEntities\n  id,title\n  1,Alice\n"
    const model = buildSemanticTimeline([
      section(1, "entities", ["entity-1"]),
      envelope(2, { type: "context_completed", tokens_used: 17, context: exact }),
    ])
    const context = model.steps.find((step) => step.kind === "context-assembly")
    if (context?.kind !== "context-assembly") throw new Error("expected context assembly")

    expect(context.summary.exactContext).toBe(exact)
    expect(context.summary.tokensUsed).toBe(17)
  })

  it("keeps metadata-only ContextCompleted content explicitly unavailable", () => {
    const model = buildSemanticTimeline([
      section(1, "entities", ["entity-1"]),
      envelope(2, { type: "context_completed", tokens_used: 17 }),
    ])
    const context = model.steps.find((step) => step.kind === "context-assembly")
    if (context?.kind !== "context-assembly") throw new Error("expected context assembly")

    expect(context.summary.exactContext).toBeNull()
  })
})
