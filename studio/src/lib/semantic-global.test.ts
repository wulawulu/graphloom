import { describe, expect, it } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { buildSemanticTimeline } from "@/lib/semantic-timeline"

function envelope(sequence: number, spanId: string, event: ExplainabilityEventPayload, parentSpanId?: string): ExplainabilityEnvelope {
  return {
    schema_version: 1,
    sequence,
    record: {
      run_id: "global-run",
      timestamp: `2026-08-20T00:00:${String(sequence).padStart(2, "0")}.000Z`,
      span_id: spanId,
      parent_span_id: parentSpanId,
      event,
    },
  }
}

function batchBuilt(sequence: number, batchIndex: number, context?: string): ExplainabilityEnvelope {
  return envelope(sequence, `batch-${batchIndex}`, {
    type: "global_map_batch_built",
    batch_index: batchIndex,
    report_count: 2,
    report_ids: [`report-${batchIndex}-a`, `report-${batchIndex}-b`],
    tokens_used: 80 + batchIndex,
    token_budget: 100,
    ...(context === undefined ? {} : { context }),
  }, "map")
}

function completeGlobalRun(): ExplainabilityEnvelope[] {
  return [
    envelope(1, "root", { type: "run_started", content_mode: "content" }),
    envelope(2, "root", { type: "query_started", method: "global", query: "Question" }),
    envelope(3, "context", { type: "global_context_built", batch_count: 2, report_count: 4 }, "root"),
    envelope(4, "map", { type: "global_map_started", batch_count: 2 }, "root"),
    batchBuilt(5, 0, "MAP CONTEXT 0\n"),
    batchBuilt(6, 1, "MAP CONTEXT 1\n"),
    envelope(7, "batch-0", { type: "llm_request_started", model_id: "map-model", prompt_tokens: 101, prompt: "MAP PROMPT 0" }, "map"),
    envelope(8, "batch-1", { type: "llm_request_started", model_id: "map-model", prompt_tokens: 102, prompt: "MAP PROMPT 1" }, "map"),
    envelope(9, "batch-1", { type: "llm_request_completed", model_id: "map-model", input_tokens: 102, output_tokens: 12, elapsed_ms: 20, response: "RAW MAP 1" }, "map"),
    envelope(10, "batch-1", { type: "global_map_points_produced", batch_index: 1, points: [{ batch_index: 1, point_index: 0, score: 8, answer: "Point 1" }] }, "map"),
    envelope(11, "batch-0", { type: "llm_request_completed", model_id: "map-model", input_tokens: 101, output_tokens: 11, elapsed_ms: 30, response: "RAW MAP 0" }, "map"),
    envelope(12, "batch-0", { type: "global_map_points_produced", batch_index: 0, points: [{ batch_index: 0, point_index: 0, score: 9, answer: "Point 0 positive" }, { batch_index: 0, point_index: 1, score: 0, answer: "Point 0 zero" }] }, "map"),
    envelope(13, "reduce", { type: "global_reduce_context_built", candidate_point_count: 3, positive_point_count: 2, selected_point_count: 1, token_budget: 100, tokens_used: 70, truncated: true, points: [{ batch_index: 0, point_index: 0, score: 9, selected: true, reason: "selected", answer: "Point 0 positive" }, { batch_index: 0, point_index: 1, score: 0, selected: false, reason: "non_positive_score", answer: "Point 0 zero" }, { batch_index: 1, point_index: 0, score: 8, selected: false, reason: "token_budget", answer: "Point 1" }], context: "REDUCE CONTEXT\n" }, "root"),
    envelope(14, "reduce", { type: "llm_request_started", model_id: "reduce-model", prompt_tokens: 120, prompt: "REDUCE PROMPT" }, "root"),
    envelope(15, "reduce", { type: "llm_request_completed", model_id: "reduce-model", input_tokens: 120, output_tokens: 30, elapsed_ms: 40, response: "RAW REDUCE RESPONSE" }, "root"),
    envelope(16, "root", { type: "run_completed", elapsed_ms: 200 }),
  ]
}

describe("Global semantic timeline", () => {
  it("builds the four Static Global decision steps from the complete event chain", () => {
    const model = buildSemanticTimeline(completeGlobalRun())

    expect(model.method).toBe("global")
    expect(model.steps.map((step) => step.title)).toEqual(["Community Context", "Map Analysis", "Evidence Reduction", "Answer Generation"])
    const community = model.steps[0]
    const map = model.steps[1]
    const reduce = model.steps[2]
    const answer = model.steps[3]
    if (community?.kind !== "community-context" || map?.kind !== "map-analysis" || reduce?.kind !== "evidence-reduction" || answer?.kind !== "global-answer-generation") throw new Error("expected Global steps")
    expect(community.summary).toMatchObject({ reportCount: 4, batchCount: 2, tokensUsed: 161 })
    expect(map.summary).toMatchObject({ batchCount: 2, analystCalls: 2, pointCount: 3, positivePointCount: 2 })
    expect(reduce.summary).toMatchObject({ candidatePointCount: 3, positivePointCount: 2, selectedPointCount: 1, nonPositiveCount: 1, tokenBudgetExcludedCount: 1, exactContext: "REDUCE CONTEXT\n" })
    expect(answer.summary).toMatchObject({ calls: 1, generated: true, model: "reduce-model", exactPrompt: "REDUCE PROMPT", rawResponse: "RAW REDUCE RESPONSE" })
  })

  it("groups out-of-order Map completions by batch span and presents batches by batch_index", () => {
    const model = buildSemanticTimeline(completeGlobalRun())
    const map = model.steps.find((step) => step.kind === "map-analysis")
    if (map?.kind !== "map-analysis") throw new Error("expected Map Analysis")

    expect(map.summary.batches.map((batch) => batch.batchIndex)).toEqual([0, 1])
    expect(map.summary.batches[0]).toMatchObject({ rawResponse: "RAW MAP 0", status: "completed" })
    expect(map.summary.batches[0]?.points.map((point) => point.answer)).toEqual(["Point 0 positive", "Point 0 zero"])
    expect(map.summary.batches[1]).toMatchObject({ rawResponse: "RAW MAP 1", status: "completed" })
  })

  it.each([
    [[], "ready"],
    [[envelope(6, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 10 }, "map")], "analyzing"],
    [[envelope(6, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 10 }, "map"), envelope(7, "batch-0", { type: "llm_request_completed", model_id: "map", input_tokens: 10, output_tokens: 2, elapsed_ms: 3 }, "map")], "response_received"],
    [[envelope(6, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 10 }, "map"), envelope(7, "batch-0", { type: "llm_request_completed", model_id: "map", input_tokens: 10, output_tokens: 2, elapsed_ms: 3 }, "map"), envelope(8, "batch-0", { type: "global_map_points_produced", batch_index: 0, points: [] }, "map")], "completed"],
  ] as const)("derives progressive batch state without inventing failure: %s", (tail, expected) => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "map", { type: "global_map_started", batch_count: 1 }, "root"),
      batchBuilt(5, 0),
      ...tail,
    ])
    const map = model.steps.find((step) => step.kind === "map-analysis")
    if (map?.kind !== "map-analysis") throw new Error("expected Map Analysis")
    expect(map.summary.batches[0]?.status).toBe(expected)
  })

  it("uses backend Reduce decision reasons without sorting or rerunning selection", () => {
    const model = buildSemanticTimeline(completeGlobalRun())
    const reduce = model.steps.find((step) => step.kind === "evidence-reduction")
    if (reduce?.kind !== "evidence-reduction") throw new Error("expected Evidence Reduction")

    expect(reduce.summary.decisions.map((point) => point.reason)).toEqual(["selected", "non_positive_score", "token_budget"])
    expect(reduce.summary.decisions.map((point) => point.score)).toEqual([9, 0, 8])
  })

  it("models the no-positive path without a Reduce LLM call", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
    ])
    const reduce = model.steps.find((step) => step.kind === "evidence-reduction")
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (reduce?.kind !== "evidence-reduction" || answer?.kind !== "global-answer-generation") throw new Error("expected Reduce and Answer")

    expect(reduce.summary.skippedReason).toBe("no_positive_points")
    expect(answer.summary).toMatchObject({ calls: 0, generated: false, noDataPathSelected: true, noDataAnswerReturned: false })
  })

  it("keeps metadata-only content unavailable instead of fabricating empty strings", () => {
    const events = completeGlobalRun().map((item) => {
      const event = { ...item.record.event }
      delete event.context
      delete event.prompt
      delete event.response
      if (Array.isArray(event.points)) event.points = event.points.map((point) => typeof point === "object" && point !== null ? { ...point, answer: undefined } : point)
      return { ...item, record: { ...item.record, event } }
    })
    const model = buildSemanticTimeline(events)
    const map = model.steps.find((step) => step.kind === "map-analysis")
    const reduce = model.steps.find((step) => step.kind === "evidence-reduction")
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (map?.kind !== "map-analysis" || reduce?.kind !== "evidence-reduction" || answer?.kind !== "global-answer-generation") throw new Error("expected Global steps")

    expect(map.summary.batches[0]).toMatchObject({ exactContext: null, exactPrompt: null, rawResponse: null })
    expect(map.summary.batches[0]?.points[0]?.answer).toBeUndefined()
    expect(reduce.summary.exactContext).toBeNull()
    expect(reduce.summary.decisions[0]?.answer).toBeUndefined()
    expect(answer.summary).toMatchObject({ exactPrompt: null, rawResponse: null })
  })

  it("starts a new coherent lifecycle after a duplicate batch fact", () => {
    const events = [
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "map", { type: "global_map_started", batch_count: 1 }, "root"),
      batchBuilt(3, 0, "old context"),
      envelope(4, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 10, prompt: "old prompt" }, "map"),
      envelope(5, "batch-0", { type: "llm_request_completed", model_id: "map", input_tokens: 10, output_tokens: 2, elapsed_ms: 3 }, "map"),
      envelope(6, "batch-0", { type: "global_map_points_produced", batch_index: 0, points: [] }, "map"),
      batchBuilt(7, 0, "latest context"),
      envelope(8, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 11, prompt: "latest prompt" }, "map"),
    ]
    const model = buildSemanticTimeline(events)
    const map = model.steps.find((step) => step.kind === "map-analysis")
    if (map?.kind !== "map-analysis") throw new Error("expected Map Analysis")

    expect(map.summary.batches[0]).toMatchObject({ exactContext: "latest context", exactPrompt: "latest prompt", rawResponse: null, points: [], status: "analyzing" })
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([4, 5, 6]))
  })

  it("binds a completed Map response to its preceding prompt and diagnoses a later backwards start", () => {
    const events = [
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "map", { type: "global_map_started", batch_count: 1 }, "root"),
      batchBuilt(3, 0, "context"),
      envelope(4, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 10, prompt: "completed prompt" }, "map"),
      envelope(5, "batch-0", { type: "llm_request_completed", model_id: "map", input_tokens: 10, output_tokens: 2, elapsed_ms: 3, response: "completed response" }, "map"),
      envelope(6, "batch-0", { type: "global_map_points_produced", batch_index: 0, points: [{ batch_index: 0, point_index: 0, score: 9, answer: "completed point" }] }, "map"),
      envelope(7, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 11, prompt: "unmatched later prompt" }, "map"),
    ]
    const model = buildSemanticTimeline(events)
    const map = model.steps.find((step) => step.kind === "map-analysis")
    if (map?.kind !== "map-analysis") throw new Error("expected Map Analysis")

    expect(map.summary.batches[0]).toMatchObject({ exactPrompt: "completed prompt", rawResponse: "completed response", status: "completed" })
    expect(map.summary.batches[0]?.points[0]?.answer).toBe("completed point")
    expect(model.diagnosticEvents.map((event) => event.sequence)).toContain(7)
  })

  it("binds a completed Reduce response to its preceding prompt", () => {
    const events = [
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 1, selected_point_count: 1, token_budget: 100, tokens_used: 20, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 9, selected: true, reason: "selected" }] }, "root"),
      envelope(3, "reduce", { type: "llm_request_started", model_id: "reduce", prompt_tokens: 20, prompt: "completed reduce prompt" }, "root"),
      envelope(4, "reduce", { type: "llm_request_completed", model_id: "reduce", input_tokens: 20, output_tokens: 5, elapsed_ms: 8, response: "completed reduce response" }, "root"),
      envelope(5, "reduce", { type: "llm_request_started", model_id: "reduce", prompt_tokens: 21, prompt: "unmatched later reduce prompt" }, "root"),
    ]
    const model = buildSemanticTimeline(events)
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (answer?.kind !== "global-answer-generation") throw new Error("expected Answer Generation")

    expect(answer.summary).toMatchObject({ generated: true, exactPrompt: "completed reduce prompt", rawResponse: "completed reduce response" })
    expect(model.diagnosticEvents.map((event) => event.sequence)).toContain(5)
  })

  it("does not carry a stale no-data branch into a newer positive Reduce lifecycle", () => {
    const events = [
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
      envelope(4, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 1, selected_point_count: 1, token_budget: 100, tokens_used: 20, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 9, selected: true, reason: "selected" }] }, "root"),
      envelope(5, "reduce", { type: "llm_request_started", model_id: "reduce", prompt_tokens: 20, prompt: "positive prompt" }, "root"),
      envelope(6, "reduce", { type: "llm_request_completed", model_id: "reduce", input_tokens: 20, output_tokens: 5, elapsed_ms: 8, response: "positive response" }, "root"),
    ]
    const model = buildSemanticTimeline(events)
    const reduce = model.steps.find((step) => step.kind === "evidence-reduction")
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (reduce?.kind !== "evidence-reduction" || answer?.kind !== "global-answer-generation") throw new Error("expected Reduce and Answer")

    expect(reduce.summary).toMatchObject({ positivePointCount: 1, skippedReason: null })
    expect(answer.summary).toMatchObject({ calls: 1, generated: true, noDataPathSelected: false, noDataAnswerReturned: false, exactPrompt: "positive prompt", rawResponse: "positive response" })
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([2, 3]))
  })

  it("does not carry an older Reduce LLM lifecycle into a newer no-data branch", () => {
    const events = [
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 1, selected_point_count: 1, token_budget: 100, tokens_used: 20, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 9, selected: true, reason: "selected" }] }, "root"),
      envelope(3, "reduce", { type: "llm_request_started", model_id: "reduce", prompt_tokens: 20 }, "root"),
      envelope(4, "reduce", { type: "llm_request_completed", model_id: "reduce", input_tokens: 20, output_tokens: 5, elapsed_ms: 8 }, "root"),
      envelope(5, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(6, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
    ]
    const model = buildSemanticTimeline(events)
    const reduce = model.steps.find((step) => step.kind === "evidence-reduction")
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (reduce?.kind !== "evidence-reduction" || answer?.kind !== "global-answer-generation") throw new Error("expected Reduce and Answer")

    expect(reduce.summary).toMatchObject({ positivePointCount: 0, skippedReason: "no_positive_points" })
    expect(answer.summary).toMatchObject({ calls: 0, generated: false, noDataPathSelected: true, noDataAnswerReturned: false, exactPrompt: null, rawResponse: null })
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([2, 3, 4]))
  })

  it("never combines facts from conflicting spans that reuse a batch_index", () => {
    const events = [
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "map", { type: "global_map_started", batch_count: 1 }, "root"),
      envelope(3, "old-batch", { type: "global_map_batch_built", batch_index: 0, report_count: 1, report_ids: ["old-report"], tokens_used: 10, token_budget: 20, context: "old context" }, "map"),
      envelope(4, "old-batch", { type: "llm_request_started", model_id: "map", prompt_tokens: 10, prompt: "old prompt" }, "map"),
      envelope(5, "old-batch", { type: "llm_request_completed", model_id: "map", input_tokens: 10, output_tokens: 2, elapsed_ms: 3, response: "old response" }, "map"),
      envelope(6, "old-batch", { type: "global_map_points_produced", batch_index: 0, points: [{ batch_index: 0, point_index: 0, score: 9, answer: "old point" }] }, "map"),
      envelope(7, "new-batch", { type: "global_map_batch_built", batch_index: 0, report_count: 1, report_ids: ["new-report"], tokens_used: 11, token_budget: 20, context: "new context" }, "map"),
      envelope(8, "new-batch", { type: "llm_request_started", model_id: "map", prompt_tokens: 11, prompt: "new prompt" }, "map"),
    ]
    const model = buildSemanticTimeline(events)
    const map = model.steps.find((step) => step.kind === "map-analysis")
    if (map?.kind !== "map-analysis") throw new Error("expected Map Analysis")

    expect(map.summary.batches[0]).toMatchObject({ spanId: "new-batch", exactContext: "new context", exactPrompt: "new prompt", rawResponse: null, status: "analyzing", points: [] })
    expect(model.diagnosticEvents.map((event) => event.record.span_id)).toEqual(expect.arrayContaining(["old-batch"]))
  })

  it("does not infer Global method from Global-looking events when QueryStarted is absent", () => {
    const model = buildSemanticTimeline([batchBuilt(1, 0)])
    expect(model.method).toBeNull()
    expect(model.steps).toEqual([])
    expect(model.diagnosticEvents).toHaveLength(1)
  })

  it("keeps not-yet-emitted stages pending instead of fabricating zero facts", () => {
    const model = buildSemanticTimeline([envelope(1, "root", { type: "query_started", method: "global" })])
    const community = model.steps.find((step) => step.kind === "community-context")
    const map = model.steps.find((step) => step.kind === "map-analysis")
    const reduce = model.steps.find((step) => step.kind === "evidence-reduction")
    if (community?.kind !== "community-context" || map?.kind !== "map-analysis" || reduce?.kind !== "evidence-reduction") throw new Error("expected Global steps")

    expect(community.summary).toMatchObject({ built: false })
    expect(community.summary.reportCount).toBeUndefined()
    expect(map.summary).toMatchObject({ started: false })
    expect(map.summary.batchCount).toBeUndefined()
    expect(reduce.summary).toMatchObject({ built: false })
    expect(reduce.summary.candidatePointCount).toBeUndefined()
  })

  it("prepends Dynamic Community Selection while reusing the four Global stages", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 2, threshold: 3, max_level: 2, keep_parent: false, use_summary: true, num_repeats: 2 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 0, source: "initial", community_ids: ["A", "C"] }, "root"),
      envelope(4, "attempt-a-0", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 0, repeat_count: 2 }, "selection"),
      envelope(5, "attempt-a-0", { type: "llm_request_started", model_id: "rating", prompt_tokens: 20, prompt: "A PROMPT\n" }, "selection"),
      envelope(6, "attempt-c-0", { type: "dynamic_community_rating_attempt_started", community_id: "C", report_id: "report-c", repeat_index: 0, repeat_count: 2 }, "selection"),
      envelope(7, "attempt-c-0", { type: "llm_request_started", model_id: "rating", prompt_tokens: 21, prompt: "C PROMPT" }, "selection"),
      envelope(8, "attempt-c-0", { type: "llm_request_completed", model_id: "rating", input_tokens: 21, output_tokens: 2, elapsed_ms: 8, response: "C RAW" }, "selection"),
      envelope(9, "attempt-a-0", { type: "llm_request_completed", model_id: "rating", input_tokens: 20, output_tokens: 3, elapsed_ms: 10, response: "A RAW" }, "selection"),
      envelope(10, "attempt-a-1", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 1, repeat_count: 2 }, "selection"),
      envelope(11, "attempt-a-1", { type: "llm_request_started", model_id: "rating", prompt_tokens: 20 }, "selection"),
      envelope(12, "attempt-a-1", { type: "llm_request_completed", model_id: "rating", input_tokens: 20, output_tokens: 3, elapsed_ms: 9 }, "selection"),
      envelope(13, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 1, source: "child_expansion", community_ids: ["B"] }, "root"),
      envelope(14, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 2, source: "fallback", community_ids: ["D"] }, "root"),
      envelope(15, "selection", { type: "dynamic_community_selection_completed", visited_count: 3, threshold_passed_count: 2, selected_count: 1, selected_community_ids: ["B"], selected_report_ids: ["report-b"], ratings: [
        { community_id: "A", report_id: "report-a", level: 0, selected_rating: 4, threshold_passed: true, selected: false },
        { community_id: "C", report_id: "report-c", level: 0, selected_rating: 2, threshold_passed: false, selected: false },
        { community_id: "B", report_id: "report-b", level: 1, selected_rating: 5, threshold_passed: true, selected: true },
      ] }, "root"),
      envelope(16, "context", { type: "global_context_built", batch_count: 0, report_count: 0 }, "root"),
    ])

    expect(model.globalVariant).toBe("dynamic")
    expect(model.steps.map((step) => step.title)).toEqual(["Community Selection", "Community Context", "Map Analysis", "Evidence Reduction", "Answer Generation"])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected Community Selection")
    expect(selection.summary).toMatchObject({ completed: true, visitedCount: 3, selectedCount: 1, threshold: 3, attemptsStarted: 3, attemptsCompleted: 3 })
    expect(selection.summary.waves.map((wave) => wave.source)).toEqual(["initial", "child_expansion", "fallback"])
    expect(selection.summary.decisions.map((decision) => [decision.community_id, decision.threshold_passed, decision.selected])).toEqual([
      ["A", true, false],
      ["C", false, false],
      ["B", true, true],
    ])
    expect(selection.summary.decisions[0]?.attempts.map((attempt) => attempt.repeatIndex)).toEqual([0, 1])
    expect(selection.summary.decisions[0]?.attempts[0]).toMatchObject({ exactPrompt: "A PROMPT\n", rawResponse: "A RAW" })
    expect(selection.summary.decisions[1]?.attempts[0]).toMatchObject({ exactPrompt: "C PROMPT", rawResponse: "C RAW" })
    expect(model.steps[1]?.kind === "community-context" && model.steps[1].summary.reportCount).toBe(0)
  })

  it("keeps Dynamic selection progressive without inventing final decisions", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 2, threshold: 3, max_level: 2, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 0, source: "initial", community_ids: ["A", "B"] }, "root"),
      envelope(4, "attempt-a", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 0, repeat_count: 1 }, "selection"),
      envelope(5, "attempt-a", { type: "llm_request_started", model_id: "rating", prompt_tokens: 20 }, "selection"),
      envelope(6, "attempt-b", { type: "dynamic_community_rating_attempt_started", community_id: "B", report_id: "report-b", repeat_index: 0, repeat_count: 1 }, "selection"),
      envelope(7, "attempt-b", { type: "llm_request_started", model_id: "rating", prompt_tokens: 20 }, "selection"),
      envelope(8, "attempt-b", { type: "llm_request_completed", model_id: "rating", input_tokens: 20, output_tokens: 2, elapsed_ms: 5 }, "selection"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary).toMatchObject({ started: true, completed: false, activeWave: 0, attemptsStarted: 2, attemptsCompleted: 1, decisions: [] })
  })

  it("anchors Dynamic replay to the latest coherent completion and diagnoses stale lifecycle", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "old-selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "old-selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 0, selected_count: 0, selected_community_ids: [], selected_report_ids: [], ratings: [{ community_id: "old", report_id: "old-report", level: 0, selected_rating: 1, threshold_passed: false, selected: false }] }, "root"),
      envelope(4, "new-selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 2, max_level: 1, keep_parent: true, use_summary: true, num_repeats: 1 }, "root"),
      envelope(5, "new-selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 1, selected_count: 1, selected_community_ids: ["new"], selected_report_ids: ["new-report"], ratings: [{ community_id: "new", report_id: "new-report", level: 0, selected_rating: 5, threshold_passed: true, selected: true }] }, "root"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary).toMatchObject({ threshold: 2, selectedCount: 1 })
    expect(selection.summary.decisions.map((decision) => decision.community_id)).toEqual(["new"])
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([2, 3]))
  })

  it("uses the latest traversal fact per wave index and diagnoses replay conflicts", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 0, source: "initial", community_ids: ["stale"] }, "root"),
      envelope(4, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 0, source: "fallback", community_ids: ["current"] }, "root"),
      envelope(5, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 0, selected_count: 0, selected_community_ids: [], selected_report_ids: [], ratings: [{ community_id: "current", report_id: "report-current", level: 1, selected_rating: 1, threshold_passed: false, selected: false }] }, "root"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary.waves).toEqual([{ waveIndex: 0, source: "fallback", communityIds: ["current"] }])
    expect(model.diagnosticEvents.map((event) => event.sequence)).toContain(3)
  })

  it("keeps attempts with conflicting wave, repeat, or report identity in Diagnostics", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 2 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 0, source: "initial", community_ids: ["A"] }, "root"),
      envelope(4, "wrong-wave", { type: "dynamic_community_rating_attempt_started", community_id: "B", report_id: "report-b", repeat_index: 0, repeat_count: 2 }, "selection"),
      envelope(5, "wrong-repeat", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 0, repeat_count: 3 }, "selection"),
      envelope(6, "wrong-report", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "other-report", repeat_index: 0, repeat_count: 2 }, "selection"),
      envelope(7, "valid", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 0, repeat_count: 2 }, "selection"),
      envelope(8, "valid", { type: "llm_request_started", model_id: "rating", prompt_tokens: 10, prompt: "valid prompt" }, "selection"),
      envelope(9, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 1, selected_count: 1, selected_community_ids: ["A"], selected_report_ids: ["report-a"], ratings: [{ community_id: "A", report_id: "report-a", level: 0, selected_rating: 5, threshold_passed: true, selected: true }] }, "root"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary.decisions[0]?.attempts).toHaveLength(1)
    expect(selection.summary.decisions[0]?.attempts[0]).toMatchObject({ spanId: "valid", exactPrompt: "valid prompt" })
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([4, 5, 6]))
  })

  it("falls back from a completion whose threshold decision contradicts SelectionStarted", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 1, selected_count: 1, selected_community_ids: ["A"], selected_report_ids: ["report-a"], ratings: [{ community_id: "A", report_id: "report-a", level: 0, selected_rating: 5, threshold_passed: true, selected: true }] }, "root"),
      envelope(4, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 0, selected_count: 0, selected_community_ids: [], selected_report_ids: [], ratings: [{ community_id: "A", report_id: "report-a", level: 0, selected_rating: 5, threshold_passed: false, selected: false }] }, "root"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary).toMatchObject({ completed: true, selectedCount: 1 })
    expect(model.diagnosticEvents.map((event) => event.sequence)).toContain(4)
  })

  it("does not let a later orphan completion displace a coherent selection lifecycle", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 1, selected_count: 1, selected_community_ids: ["A"], selected_report_ids: ["report-a"], ratings: [{ community_id: "A", report_id: "report-a", level: 0, selected_rating: 5, threshold_passed: true, selected: true }] }, "root"),
      envelope(4, "orphan", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 0, selected_count: 0, selected_community_ids: [], selected_report_ids: [], ratings: [{ community_id: "orphan", report_id: "report-orphan", level: 0, selected_rating: 1, threshold_passed: false, selected: false }] }, "root"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary.decisions.map((decision) => decision.community_id)).toEqual(["A"])
    expect(model.diagnosticEvents.map((event) => event.sequence)).toContain(4)
  })

  it("does not let a coherent selection lifecycle under a foreign parent displace the Query lifecycle", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 1, selected_count: 1, selected_community_ids: ["A"], selected_report_ids: ["report-a"], ratings: [{ community_id: "A", report_id: "report-a", level: 0, selected_rating: 5, threshold_passed: true, selected: true }] }, "root"),
      envelope(4, "foreign-selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "foreign-root"),
      envelope(5, "foreign-selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 0, selected_count: 0, selected_community_ids: [], selected_report_ids: [], ratings: [{ community_id: "foreign", report_id: "report-foreign", level: 0, selected_rating: 1, threshold_passed: false, selected: false }] }, "foreign-root"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary.decisions.map((decision) => decision.community_id)).toEqual(["A"])
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([4, 5]))
  })

  it("keeps the first root QueryStarted as the immutable lifecycle anchor", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 1, selected_count: 1, selected_community_ids: ["A"], selected_report_ids: ["report-a"], ratings: [{ community_id: "A", report_id: "report-a", level: 0, selected_rating: 5, threshold_passed: true, selected: true }] }, "root"),
      envelope(4, "orphan-root", { type: "query_started", method: "global" }),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary.decisions.map((decision) => decision.community_id)).toEqual(["A"])
  })

  it("does not mix an older completed rating attempt with a newer duplicate attempt", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 1, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "old-attempt", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 0, repeat_count: 1 }, "selection"),
      envelope(4, "old-attempt", { type: "llm_request_started", model_id: "rating", prompt_tokens: 10, prompt: "old prompt" }, "selection"),
      envelope(5, "old-attempt", { type: "llm_request_completed", model_id: "rating", input_tokens: 10, output_tokens: 1, elapsed_ms: 2, response: "old response" }, "selection"),
      envelope(6, "new-attempt", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 0, repeat_count: 1 }, "selection"),
      envelope(7, "new-attempt", { type: "llm_request_started", model_id: "rating", prompt_tokens: 11, prompt: "new prompt" }, "selection"),
      envelope(8, "selection", { type: "dynamic_community_selection_completed", visited_count: 1, threshold_passed_count: 1, selected_count: 1, selected_community_ids: ["A"], selected_report_ids: ["report-a"], ratings: [{ community_id: "A", report_id: "report-a", level: 0, selected_rating: 5, threshold_passed: true, selected: true }] }, "root"),
    ])
    const selection = model.steps[0]
    if (selection?.kind !== "community-selection") throw new Error("expected selection")
    expect(selection.summary.decisions[0]?.attempts[0]).toMatchObject({ spanId: "new-attempt", status: "rating", exactPrompt: "new prompt", rawResponse: null })
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([3, 4, 5]))
  })

  it.each([
    ["run_completed", true],
    ["run_failed", false],
  ] as const)("only reports a returned no-data answer after a successful terminal: %s", (terminalType, returned) => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
      envelope(4, "root", terminalType === "run_completed" ? { type: terminalType, elapsed_ms: 4 } : { type: terminalType, error_kind: "query_completion", message: "failed" }),
    ])
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (answer?.kind !== "global-answer-generation") throw new Error("expected answer")
    expect(answer.summary.noDataPathSelected).toBe(true)
    expect(answer.summary.noDataAnswerReturned).toBe(returned)
  })

  it("does not accept a foreign-span completion as proof that the no-data answer returned", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
      envelope(4, "foreign", { type: "run_completed", elapsed_ms: 4 }),
    ])
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (answer?.kind !== "global-answer-generation") throw new Error("expected answer")
    expect(answer.summary).toMatchObject({ noDataPathSelected: true, noDataAnswerReturned: false })
  })

  it("does not accept a child QueryStarted and matching terminal as root completion proof", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
      envelope(4, "child", { type: "query_started", method: "global" }, "root"),
      envelope(5, "child", { type: "run_completed", elapsed_ms: 4 }),
    ])
    const answer = model.steps.find((step) => step.kind === "global-answer-generation")
    if (answer?.kind !== "global-answer-generation") throw new Error("expected answer")
    expect(answer.summary).toMatchObject({ noDataPathSelected: true, noDataAnswerReturned: false })
  })
})
