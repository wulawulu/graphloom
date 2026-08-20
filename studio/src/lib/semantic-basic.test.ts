import { describe, expect, it } from "vitest"

import type { ExplainabilityCandidate, ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { buildSemanticTimeline, type SemanticTimelineModel } from "@/lib/semantic-timeline"

function envelope(sequence: number, spanId: string, event: ExplainabilityEventPayload, parentSpanId?: string): ExplainabilityEnvelope {
  return {
    schema_version: 1,
    sequence,
    record: {
      run_id: "basic-run",
      timestamp: new Date(sequence * 10).toISOString(),
      span_id: spanId,
      ...(parentSpanId === undefined ? {} : { parent_span_id: parentSpanId }),
      event,
    },
  }
}

function candidate(id: string, rank: number, selected = false, reason = "ann_result"): ExplainabilityCandidate {
  return { id, short_id: id.toLowerCase(), record_type: "text_unit", score: 1 - rank / 10, rank, selected, reason }
}

function fullBasicRun(): ExplainabilityEnvelope[] {
  const retrieved = [candidate("C", 1), candidate("A", 2), candidate("B", 3)]
  const filtered = [candidate("A", 2, true), candidate("B", 3, false, "token_budget"), candidate("C", 1, false, "token_budget")]
  return [
    envelope(1, "root", { type: "query_started", method: "basic", query: "question" }),
    envelope(2, "embedding", { type: "embedding_started", model_id: "embed", input: "question" }, "root"),
    envelope(3, "embedding", { type: "embedding_completed", model_id: "embed", prompt_tokens: 2, dimensions: 2 }, "root"),
    envelope(4, "retrieval", { type: "candidates_retrieved", record_type: "text_unit", candidates: retrieved }, "root"),
    envelope(5, "context", { type: "context_budget_allocated", total_token_budget: 20, sections: [{ section: "sources", token_budget: 20 }] }, "root"),
    envelope(6, "retrieval", { type: "candidates_filtered", record_type: "text_unit", candidates: filtered }, "root"),
    envelope(7, "context", { type: "context_section_built", section: { section: "sources", token_budget: 20, tokens_used: 12, candidate_count: 3, selected_count: 1, truncated: true, selected_record_ids: ["A"] } }, "root"),
    envelope(8, "context", { type: "context_completed", tokens_used: 12, context: "id|text\nA|exact\n" }, "root"),
    envelope(9, "llm", { type: "llm_request_started", model_id: "chat", prompt_tokens: 30, prompt: "BASIC PROMPT" }, "root"),
    envelope(10, "llm", { type: "llm_request_completed", model_id: "chat", input_tokens: 30, output_tokens: 4, elapsed_ms: 25, response: "RAW RESPONSE" }, "root"),
    envelope(11, "root", { type: "run_completed", elapsed_ms: 100 }),
  ]
}

function retrievalStep(model: SemanticTimelineModel) {
  const step = model.steps.find((value) => value.kind === "text-retrieval")
  if (step?.kind !== "text-retrieval") throw new Error("Text Retrieval step")
  return step
}

function contextStep(model: SemanticTimelineModel) {
  const step = model.steps.find((value) => value.kind === "basic-context-assembly")
  if (step?.kind !== "basic-context-assembly") throw new Error("Basic Context step")
  return step
}

function answerStep(model: SemanticTimelineModel) {
  const step = model.steps.find((value) => value.kind === "basic-answer-generation")
  if (step?.kind !== "basic-answer-generation") throw new Error("Basic Answer step")
  return step
}

describe("Basic semantic timeline", () => {
  it("builds exactly Text Retrieval, Context Assembly, and Answer Generation", () => {
    const model = buildSemanticTimeline(fullBasicRun())
    expect(model.method).toBe("basic")
    expect(model.steps.map((step) => step.title)).toEqual(["Text Retrieval", "Context Assembly", "Answer Generation"])
  })

  it("keeps ANN provider order distinct from effective table order and consumes backend decisions", () => {
    const model = buildSemanticTimeline(fullBasicRun())
    const retrieval = retrievalStep(model)
    const context = contextStep(model)
    expect(retrieval.summary.candidates.map((value) => value.id)).toEqual(["C", "A", "B"])
    expect(context.summary.candidates.map((value) => value.id)).toEqual(["A", "B", "C"])
    expect(context.summary.candidates.map((value) => [value.selected, value.reason])).toEqual([
      [true, "ann_result"],
      [false, "token_budget"],
      [false, "token_budget"],
    ])
    expect(context.summary.exactContext).toBe("id|text\nA|exact\n")
  })

  it("tracks progressive retrieval and answer states without inventing failure", () => {
    const base = fullBasicRun()
    const states = [
      [base.slice(0, 1), "waiting"],
      [base.slice(0, 2), "embedding"],
      [base.slice(0, 3), "embedding_ready"],
      [base.slice(0, 4), "retrieved"],
    ] as const
    for (const [events, status] of states) {
      expect(retrievalStep(buildSemanticTimeline(events)).summary.status).toBe(status)
    }
    expect(answerStep(buildSemanticTimeline(base.slice(0, 9))).summary.status).toBe("generating")
    expect(answerStep(buildSemanticTimeline(base.slice(0, 10))).summary.status).toBe("generated")
  })

  it("represents empty-query skip without an ANN result count", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "basic" }),
      envelope(2, "retrieval", { type: "basic_retrieval_skipped", reason: "empty_query" }, "root"),
    ])
    const retrieval = retrievalStep(model)
    expect(retrieval.summary.status).toBe("skipped")
    expect(retrieval.summary.candidates).toEqual([])
  })

  it("keeps metadata content absent instead of fabricating empty text", () => {
    const events = fullBasicRun().map((item) => {
      const event = { ...item.record.event }
      delete event.input
      delete event.context
      delete event.prompt
      delete event.response
      return { ...item, record: { ...item.record, event } }
    })
    const model = buildSemanticTimeline(events)
    const context = contextStep(model)
    const answer = answerStep(model)
    expect(context.summary.exactContext).toBeNull()
    expect(answer.summary.exactPrompt).toBeNull()
    expect(answer.summary.rawResponse).toBeNull()
  })

  it("uses the first root QueryStarted and leaves foreign or backwards lifecycle facts in Diagnostics", () => {
    const model = buildSemanticTimeline([
      ...fullBasicRun(),
      envelope(12, "child-query", { type: "query_started", method: "basic" }, "root"),
      envelope(13, "foreign-embedding", { type: "embedding_started", model_id: "wrong", input: "wrong" }, "foreign-root"),
      envelope(14, "llm", { type: "llm_request_started", model_id: "wrong-late", prompt_tokens: 999, prompt: "wrong" }, "root"),
      envelope(15, "orphan-root", { type: "query_started", method: "basic" }),
    ])
    const answer = answerStep(model)
    expect(answer.summary.model).toBe("chat")
    expect(answer.summary.exactPrompt).toBe("BASIC PROMPT")
    expect(model.diagnosticEvents.map((event) => event.sequence)).toEqual(expect.arrayContaining([12, 13, 14, 15]))
  })

  it("does not let a later duplicate retrieval erase a stronger filtered decision", () => {
    const model = buildSemanticTimeline([
      envelope(1, "root", { type: "query_started", method: "basic" }),
      envelope(2, "retrieval", { type: "candidates_retrieved", record_type: "text_unit", candidates: [candidate("A", 1)] }, "root"),
      envelope(3, "retrieval", { type: "candidates_filtered", record_type: "text_unit", candidates: [candidate("A", 1, true)] }, "root"),
      envelope(4, "retrieval", { type: "candidates_retrieved", record_type: "text_unit", candidates: [candidate("WRONG", 1)] }, "root"),
      envelope(5, "context", { type: "context_budget_allocated", total_token_budget: 20, sections: [{ section: "sources", token_budget: 20 }] }, "root"),
      envelope(6, "context", { type: "context_section_built", section: { section: "sources", token_budget: 20, tokens_used: 10, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: ["A"] } }, "root"),
      envelope(7, "context", { type: "context_completed", tokens_used: 10 }, "root"),
    ])
    expect(retrievalStep(model).summary.candidates.map((value) => value.id)).toEqual(["A"])
    expect(contextStep(model).summary.candidates.map((value) => value.id)).toEqual(["A"])
    expect(model.diagnosticEvents.map((event) => event.sequence)).toContain(4)
  })
})
