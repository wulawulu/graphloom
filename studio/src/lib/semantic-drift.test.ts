import { describe, expect, it } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { buildSemanticTimeline, type SemanticTimelineModel } from "@/lib/semantic-timeline"

function envelope(sequence: number, spanId: string, event: ExplainabilityEventPayload, parentSpanId?: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "drift-run", timestamp: new Date(sequence * 10).toISOString(), span_id: spanId, ...(parentSpanId === undefined ? {} : { parent_span_id: parentSpanId }), event } }
}

function fullDriftRun(): ExplainabilityEnvelope[] {
  return [
    envelope(1, "root", { type: "query_started", method: "drift", query: "ROOT QUERY" }),
    envelope(2, "hyde", { type: "drift_hyde_started", template_report_id: "report-stable", template_short_id: "R-1", template_community_id: "community-1", template_index: 1, report_count: 2 }, "root"),
    envelope(3, "hyde", { type: "llm_request_started", model_id: "chat", prompt_tokens: 10, prompt: "HYDE PROMPT" }, "root"),
    envelope(4, "hyde", { type: "llm_request_completed", model_id: "chat", input_tokens: 10, output_tokens: 3, elapsed_ms: 5, response: "HYDE OUTPUT" }, "root"),
    envelope(5, "hyde", { type: "drift_hyde_completed", used_original_query: false }, "root"),
    envelope(6, "embedding", { type: "embedding_started", model_id: "embed", input: "HYDE OUTPUT" }, "root"),
    envelope(7, "embedding", { type: "embedding_completed", model_id: "embed", prompt_tokens: 3, dimensions: 2 }, "root"),
    envelope(8, "ranking", { type: "drift_reports_ranked", reports: [
      { report_id: "report-z", short_id: "Z", community_id: "community-z", similarity: 0.6, rank: 1 },
      { report_id: "report-a", short_id: "A", community_id: "community-a", similarity: 0.9, rank: 2 },
    ] }, "root"),
    envelope(9, "primer", { type: "drift_primer_started", fold_count: 3, ranked_report_count: 2 }, "root"),
    envelope(10, "fold-0", { type: "drift_primer_fold_started", fold_index: 0, fold_count: 3, report_ids: ["report-z"] }, "primer"),
    envelope(11, "fold-1", { type: "drift_primer_fold_started", fold_index: 1, fold_count: 3, report_ids: ["report-a"] }, "primer"),
    envelope(12, "fold-2", { type: "drift_primer_fold_started", fold_index: 2, fold_count: 3, report_ids: [] }, "primer"),
    envelope(13, "fold-0", { type: "llm_request_started", model_id: "chat", prompt_tokens: 20, prompt: "FOLD 0" }, "primer"),
    envelope(14, "fold-1", { type: "llm_request_started", model_id: "chat", prompt_tokens: 20, prompt: "FOLD 1" }, "primer"),
    envelope(15, "fold-2", { type: "llm_request_started", model_id: "chat", prompt_tokens: 5, prompt: "FOLD 2 EMPTY" }, "primer"),
    envelope(16, "fold-2", { type: "llm_request_completed", model_id: "chat", input_tokens: 5, output_tokens: 2, elapsed_ms: 2, response: "RAW 2" }, "primer"),
    envelope(17, "fold-2", { type: "drift_primer_fold_completed", fold_index: 2, score: 30, follow_up_count: 0, intermediate_answer: "A2", follow_up_queries: [] }, "primer"),
    envelope(18, "fold-0", { type: "llm_request_completed", model_id: "chat", input_tokens: 20, output_tokens: 4, elapsed_ms: 8, response: "RAW 0" }, "primer"),
    envelope(19, "fold-0", { type: "drift_primer_fold_completed", fold_index: 0, score: 90, follow_up_count: 2, intermediate_answer: "A0", follow_up_queries: ["same", "same"] }, "primer"),
    envelope(20, "fold-1", { type: "llm_request_completed", model_id: "chat", input_tokens: 20, output_tokens: 4, elapsed_ms: 9, response: "RAW 1" }, "primer"),
    envelope(21, "fold-1", { type: "drift_primer_fold_completed", fold_index: 1, score: 60, follow_up_count: 1, intermediate_answer: "A1", follow_up_queries: ["other"] }, "primer"),
    envelope(22, "primer", { type: "drift_primer_completed", score: 123.4, root_action_id: 0, follow_up_count: 3, follow_up_action_ids: [1, 1, 2], answer: "PRIMER AGGREGATE", follow_up_queries: ["same", "same", "other"] }, "root"),
    envelope(23, "exploration", { type: "drift_exploration_started", max_depth: 2, selection_limit: 2, root_action_id: 0 }, "root"),
    envelope(24, "exploration", { type: "drift_depth_actions_selected", depth_index: 0, candidate_action_ids: [1, 2], selected_action_ids: [2, 1], selection_limit: 2 }, "root"),
    envelope(25, "attempt-2-0", { type: "drift_action_attempt_started", depth_index: 0, action_id: 2, query: "other" }, "exploration"),
    envelope(26, "attempt-1-0", { type: "drift_action_attempt_started", depth_index: 0, action_id: 1, query: "same" }, "exploration"),
    envelope(27, "attempt-2-0", { type: "drift_action_context_built", action_id: 2, context: "CONTEXT 2A" }, "exploration"),
    envelope(28, "attempt-1-0", { type: "drift_action_context_built", action_id: 1, context: "CONTEXT 1" }, "exploration"),
    envelope(29, "attempt-2-0", { type: "llm_request_started", model_id: "chat", prompt_tokens: 30, prompt: "ACTION PROMPT" }, "exploration"),
    envelope(30, "attempt-1-0", { type: "llm_request_started", model_id: "chat", prompt_tokens: 30, prompt: "ACTION PROMPT" }, "exploration"),
    envelope(31, "attempt-1-0", { type: "llm_request_completed", model_id: "chat", input_tokens: 30, output_tokens: 5, elapsed_ms: 3, response: "RAW ACTION 1" }, "exploration"),
    envelope(32, "attempt-2-0", { type: "llm_request_completed", model_id: "chat", input_tokens: 30, output_tokens: 5, elapsed_ms: 7, response: "RAW ACTION 2A" }, "exploration"),
    envelope(33, "attempt-2-0", { type: "drift_action_attempt_completed", depth_index: 0, action_id: 2, answer_present: false, answer_non_empty: false, follow_up_count: 1, target_action_ids: [3], follow_up_queries: ["shared"] }, "exploration"),
    envelope(34, "attempt-1-0", { type: "drift_action_attempt_completed", depth_index: 0, action_id: 1, answer_present: true, answer_non_empty: false, score: 50, follow_up_count: 1, target_action_ids: [3], answer: "", follow_up_queries: ["shared"] }, "exploration"),
    envelope(35, "exploration", { type: "drift_depth_actions_selected", depth_index: 1, candidate_action_ids: [2, 3], selected_action_ids: [2], selection_limit: 2 }, "root"),
    envelope(36, "attempt-2-1", { type: "drift_action_attempt_started", depth_index: 1, action_id: 2, query: "other" }, "exploration"),
    envelope(37, "attempt-2-1", { type: "drift_action_context_built", action_id: 2, context: "CONTEXT 2B" }, "exploration"),
    envelope(38, "attempt-2-1", { type: "llm_request_started", model_id: "chat", prompt_tokens: 31, prompt: "ACTION PROMPT" }, "exploration"),
    envelope(39, "attempt-2-1", { type: "llm_request_completed", model_id: "chat", input_tokens: 31, output_tokens: 1, elapsed_ms: 4, response: "RAW SPACES" }, "exploration"),
    envelope(40, "attempt-2-1", { type: "drift_action_attempt_completed", depth_index: 1, action_id: 2, answer_present: true, answer_non_empty: true, score: 70, follow_up_count: 0, target_action_ids: [], answer: "   ", follow_up_queries: [] }, "exploration"),
    envelope(41, "reduce", { type: "drift_reduce_context_built", node_count: 4, edge_count: 5, included_answer_count: 2, included_action_ids: [0, 2], state_context: "{\"exact\":true}", reduce_context: "['PRIMER AGGREGATE', '   ']" }, "root"),
    envelope(42, "reduce", { type: "llm_request_started", model_id: "chat", prompt_tokens: 40, prompt: "REDUCE PROMPT" }, "root"),
    envelope(43, "reduce", { type: "llm_request_completed", model_id: "chat", input_tokens: 40, output_tokens: 8, elapsed_ms: 10, response: "FINAL RAW" }, "root"),
    envelope(44, "root", { type: "run_completed", elapsed_ms: 200 }),
  ]
}

function driftStep<K extends "drift-primer-ranking" | "drift-exploration" | "drift-final-synthesis">(model: SemanticTimelineModel, kind: K): Extract<SemanticTimelineModel["steps"][number], { kind: K }> {
  const step = model.steps.find((candidate) => candidate.kind === kind)
  if (step?.kind !== kind) throw new Error(`Missing ${kind}`)
  return step as Extract<SemanticTimelineModel["steps"][number], { kind: K }>
}

describe("DRIFT semantic timeline", () => {
  it("builds exactly the three DRIFT semantic steps", () => {
    const model = buildSemanticTimeline(fullDriftRun())
    expect(model.method).toBe("drift")
    expect(model.steps.map((step) => step.title)).toEqual(["Primer & Ranking", "Exploration", "Final Synthesis"])
  })

  it("preserves backend ranking and aggregate facts without recomputing them", () => {
    const primer = driftStep(buildSemanticTimeline(fullDriftRun()), "drift-primer-ranking")
    expect(primer.summary.rankedReports.map((report) => report.report_id)).toEqual(["report-z", "report-a"])
    expect(primer.summary.rankedReports.map((report) => report.similarity)).toEqual([0.6, 0.9])
    expect(primer.summary.aggregate?.score).toBe(123.4)
    expect(primer.summary.aggregate?.followUpCount).toBe(3)
    expect(primer.summary.aggregate?.followUpActionIds).toEqual([1, 1, 2])
  })

  it("groups out-of-order fold completion by span and sorts by fold index including empty folds", () => {
    const primer = driftStep(buildSemanticTimeline(fullDriftRun()), "drift-primer-ranking")
    expect(primer.summary.folds.map((fold) => [fold.foldIndex, fold.score, fold.reportIds])).toEqual([
      [0, 90, ["report-z"]],
      [1, 60, ["report-a"]],
      [2, 30, []],
    ])
    expect(primer.summary.folds[2]?.rawResponse).toBe("RAW 2")
  })

  it("records random depth decisions and repeated attempts under one action node", () => {
    const exploration = driftStep(buildSemanticTimeline(fullDriftRun()), "drift-exploration")
    expect(exploration.summary.depths[0]).toMatchObject({ candidateActionIds: [1, 2], selectedActionIds: [2, 1] })
    const action = exploration.summary.nodes.find((node) => node.actionId === 2)
    expect(action?.attempts.map((attempt) => [attempt.depthIndex, attempt.status])).toEqual([[0, "incomplete"], [1, "completed"]])
    expect(action?.status).toBe("completed")
  })

  it("preserves duplicate edges and a single shared node with multiple parents", () => {
    const exploration = driftStep(buildSemanticTimeline(fullDriftRun()), "drift-exploration")
    expect(exploration.summary.edges.map((edge) => [edge.sourceActionId, edge.targetActionId])).toEqual([
      [0, 1], [0, 1], [0, 2], [2, 3], [1, 3],
    ])
    expect(exploration.summary.nodes.filter((node) => node.actionId === 3)).toHaveLength(1)
    expect(exploration.summary.nodes.find((node) => node.actionId === 3)?.status).toBe("not_explored")
  })

  it("keeps empty and whitespace answers distinct and trusts backend Reduce IDs", () => {
    const model = buildSemanticTimeline(fullDriftRun())
    const exploration = driftStep(model, "drift-exploration")
    const synthesis = driftStep(model, "drift-final-synthesis")
    expect(exploration.summary.nodes.find((node) => node.actionId === 1)?.status).toBe("completed_empty")
    expect(exploration.summary.nodes.find((node) => node.actionId === 2)?.attempts.at(-1)?.answer).toBe("   ")
    expect(synthesis.summary.includedActionIds).toEqual([0, 2])
    expect(synthesis.summary.exactReduceContext).toBe("['PRIMER AGGREGATE', '   ']")
    expect(synthesis.summary.exactStateContext).toBe("{\"exact\":true}")
  })

  it("makes empty-HyDE fallback explicit and reads the effective real embedding input", () => {
    const events = fullDriftRun().map((item) => item.record.event.type === "drift_hyde_completed"
      ? { ...item, record: { ...item.record, event: { type: "drift_hyde_completed", used_original_query: true } as const } }
      : item.record.event.type === "embedding_started"
        ? { ...item, record: { ...item.record, event: { type: "embedding_started", model_id: "embed", input: "ROOT QUERY" } as const } }
        : item)
    const primer = driftStep(buildSemanticTimeline(events), "drift-primer-ranking")
    expect(primer.summary.hyde?.usedOriginalQuery).toBe(true)
    expect(primer.summary.hyde?.effectiveQuery).toBe("ROOT QUERY")
  })

  it("keeps metadata content absent rather than fabricating semantic text", () => {
    const contentFields = new Set(["query", "input", "prompt", "response", "intermediate_answer", "follow_up_queries", "answer", "context", "state_context", "reduce_context"])
    const metadata = fullDriftRun().map((item) => {
      const event = { ...item.record.event }
      contentFields.forEach((field) => delete event[field])
      return { ...item, record: { ...item.record, event } }
    })
    const model = buildSemanticTimeline(metadata)
    const primer = driftStep(model, "drift-primer-ranking")
    const exploration = driftStep(model, "drift-exploration")
    const synthesis = driftStep(model, "drift-final-synthesis")
    expect(primer.summary.hyde?.exactPrompt).toBeNull()
    expect(primer.summary.aggregate?.followUpQueries).toBeNull()
    expect(exploration.summary.attempts[0]?.query).toBeNull()
    expect(exploration.summary.attempts[0]?.context).toBeNull()
    expect(synthesis.summary.exactStateContext).toBeNull()
    expect(synthesis.summary.exactReduceContext).toBeNull()
  })

  it("uses immutable root and stage topology, cutting off duplicates and foreign facts", () => {
    const events = fullDriftRun()
    events.splice(5, 0,
      envelope(4.1, "foreign-hyde", { type: "drift_hyde_started", template_report_id: "WRONG", template_short_id: "WRONG", template_community_id: "WRONG", template_index: 0, report_count: 1 }, "foreign-root"),
      envelope(4.2, "hyde", { type: "llm_request_completed", model_id: "wrong", input_tokens: 1, output_tokens: 1, elapsed_ms: 1, response: "WRONG" }, "root"),
    )
    events.push(
      envelope(45, "late-reduce", { type: "drift_reduce_context_built", node_count: 999, edge_count: 999, included_answer_count: 0, included_action_ids: [] }, "root"),
      envelope(46, "other-root", { type: "query_started", method: "drift", query: "OTHER" }),
    )
    const model = buildSemanticTimeline(events)
    const primer = driftStep(model, "drift-primer-ranking")
    const synthesis = driftStep(model, "drift-final-synthesis")
    expect(primer.summary.hyde?.templateReportId).toBe("report-stable")
    expect(primer.summary.hyde?.rawResponse).toBe("HYDE OUTPUT")
    expect(synthesis.summary.nodeCount).toBe(4)
    expect(model.diagnosticEvents.map((item) => item.sequence)).toEqual(expect.arrayContaining([4.1, 4.2, 45, 46]))
  })

  it("returns progressive three-step models without inventing final aggregates", () => {
    const events = fullDriftRun()
    const primer = driftStep(buildSemanticTimeline(events.filter((item) => item.sequence <= 9)), "drift-primer-ranking")
    const exploration = driftStep(buildSemanticTimeline(events.filter((item) => item.sequence <= 24)), "drift-exploration")
    expect(primer.summary.status).toBe("primer")
    expect(primer.summary.aggregate).toBeNull()
    expect(exploration.summary.activeDepth).toBe(0)
    expect(exploration.summary.depths[0]?.selectedActionIds).toEqual([2, 1])
  })
})
