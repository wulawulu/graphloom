import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"

afterEach(cleanup)

function envelope(sequence: number, spanId: string, event: ExplainabilityEventPayload, parentSpanId?: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "global-run", timestamp: "2026-08-20T00:00:00Z", span_id: spanId, parent_span_id: parentSpanId, event } }
}

function globalEvents(content = true): ExplainabilityEnvelope[] {
  return [
    envelope(1, "root", { type: "run_started", content_mode: content ? "content" : "metadata" }),
    envelope(2, "root", { type: "query_started", method: "global" }),
    envelope(3, "context", { type: "global_context_built", batch_count: 2, report_count: 3 }, "root"),
    envelope(4, "map", { type: "global_map_started", batch_count: 2 }, "root"),
    envelope(5, "batch-0", { type: "global_map_batch_built", batch_index: 0, report_count: 2, report_ids: ["report-a", "report-b"], tokens_used: 80, token_budget: 100, ...(content ? { context: "MAP CONTEXT 0\n  preserve whitespace\n" } : {}) }, "map"),
    envelope(6, "batch-1", { type: "global_map_batch_built", batch_index: 1, report_count: 1, report_ids: ["report-c"], tokens_used: 70, token_budget: 100, ...(content ? { context: "MAP CONTEXT 1" } : {}) }, "map"),
    envelope(7, "batch-0", { type: "llm_request_started", model_id: "map-model", prompt_tokens: 90, ...(content ? { prompt: "MAP PROMPT 0" } : {}) }, "map"),
    envelope(8, "batch-0", { type: "llm_request_completed", model_id: "map-model", input_tokens: 90, output_tokens: 10, elapsed_ms: 20, ...(content ? { response: "RAW MAP RESPONSE 0" } : {}) }, "map"),
    envelope(9, "batch-0", { type: "global_map_points_produced", batch_index: 0, points: [{ batch_index: 0, point_index: 0, score: 9, ...(content ? { answer: "PARSED POINT ANSWER" } : {}) }, { batch_index: 0, point_index: 1, score: 0, ...(content ? { answer: "ZERO POINT" } : {}) }] }, "map"),
    envelope(10, "batch-1", { type: "llm_request_started", model_id: "map-model", prompt_tokens: 80 }, "map"),
    envelope(11, "reduce", { type: "global_reduce_context_built", candidate_point_count: 3, positive_point_count: 2, selected_point_count: 1, token_budget: 100, tokens_used: 60, truncated: true, points: [{ batch_index: 0, point_index: 0, score: 9, selected: true, reason: "selected", ...(content ? { answer: "PARSED POINT ANSWER" } : {}) }, { batch_index: 0, point_index: 1, score: 0, selected: false, reason: "non_positive_score", ...(content ? { answer: "ZERO POINT" } : {}) }, { batch_index: 1, point_index: 0, score: 7, selected: false, reason: "token_budget", ...(content ? { answer: "BUDGET POINT" } : {}) }], ...(content ? { context: "REDUCE CONTEXT\n  exact\n" } : {}) }, "root"),
    envelope(12, "reduce", { type: "llm_request_started", model_id: "reduce-model", prompt_tokens: 110, ...(content ? { prompt: "REDUCE PROMPT" } : {}) }, "root"),
    envelope(13, "reduce", { type: "llm_request_completed", model_id: "reduce-model", input_tokens: 110, output_tokens: 20, elapsed_ms: 30, ...(content ? { response: "RAW REDUCE RESPONSE" } : {}) }, "root"),
    envelope(14, "root", { type: "run_completed", elapsed_ms: 200 }),
  ]
}

function dynamicEvents(content = true): ExplainabilityEnvelope[] {
  return [
    envelope(1, "root", { type: "query_started", method: "global" }),
    envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 2, threshold: 3, max_level: 2, keep_parent: false, use_summary: true, num_repeats: 1 }, "root"),
    envelope(3, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 0, source: "initial", community_ids: ["A", "C"] }, "root"),
    envelope(4, "attempt-a", { type: "dynamic_community_rating_attempt_started", community_id: "A", report_id: "report-a", repeat_index: 0, repeat_count: 1 }, "selection"),
    envelope(5, "attempt-a", { type: "llm_request_started", model_id: "rating-model", prompt_tokens: 30, ...(content ? { prompt: "RATING PROMPT A\n  exact" } : {}) }, "selection"),
    envelope(6, "attempt-c", { type: "dynamic_community_rating_attempt_started", community_id: "C", report_id: "report-c", repeat_index: 0, repeat_count: 1 }, "selection"),
    envelope(7, "attempt-c", { type: "llm_request_started", model_id: "rating-model", prompt_tokens: 31 }, "selection"),
    envelope(8, "attempt-c", { type: "llm_request_completed", model_id: "rating-model", input_tokens: 31, output_tokens: 2, elapsed_ms: 8, ...(content ? { response: "RAW RATING C" } : {}) }, "selection"),
    envelope(9, "attempt-a", { type: "llm_request_completed", model_id: "rating-model", input_tokens: 30, output_tokens: 3, elapsed_ms: 10, ...(content ? { response: "RAW RATING A" } : {}) }, "selection"),
    envelope(10, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 1, source: "child_expansion", community_ids: ["B"] }, "root"),
    envelope(11, "selection", { type: "dynamic_community_traversal_wave_started", wave_index: 2, source: "fallback", community_ids: ["D"] }, "root"),
    envelope(12, "selection", { type: "dynamic_community_selection_completed", visited_count: 3, threshold_passed_count: 2, selected_count: 1, selected_community_ids: ["B"], selected_report_ids: ["report-b"], ratings: [
      { community_id: "A", report_id: "report-a", level: 0, selected_rating: 4, threshold_passed: true, selected: false },
      { community_id: "C", report_id: "report-c", level: 0, selected_rating: 2, threshold_passed: false, selected: false },
      { community_id: "B", report_id: "report-b", level: 1, selected_rating: 5, threshold_passed: true, selected: true },
    ] }, "root"),
    envelope(13, "context", { type: "global_context_built", batch_count: 0, report_count: 0 }, "root"),
  ]
}

function renderGlobal(events = globalEvents(), runId = "global-run"): ReturnType<typeof render> {
  return render(<Timeline runId={runId} envelopes={events} streamStatus="closed" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} />)
}

describe("Global Timeline", () => {
  it("renders exactly four Global semantic steps and keeps lifecycle events in Diagnostics", async () => {
    const user = userEvent.setup()
    renderGlobal()

    expect(screen.getAllByRole("article").map((article) => article.getAttribute("aria-label")).filter(Boolean)).toEqual(["Community Context", "Map Analysis", "Evidence Reduction", "Answer Generation"])
    expect(screen.queryByText("Global context built")).not.toBeInTheDocument()
    await user.click(screen.getByText(/Diagnostics \/ Raw events/))
    expect(screen.getByText("Run started")).toBeInTheDocument()
    expect(screen.getByText("Query started")).toBeInTheDocument()
    expect(screen.getByText("Run completed")).toBeInTheDocument()
  })

  it("renders Dynamic Global as five steps with waves, three decision states, and coherent attempts", async () => {
    const user = userEvent.setup()
    renderGlobal(dynamicEvents())
    expect(screen.getAllByRole("article").map((article) => article.getAttribute("aria-label")).filter(Boolean)).toEqual(["Community Selection", "Community Context", "Map Analysis", "Evidence Reduction", "Answer Generation"])
    const selection = screen.getByRole("article", { name: "Community Selection" })
    expect(within(selection).getByText(/3 communities rated · 1 selected · threshold 3/)).toBeInTheDocument()
    expect(within(selection).getByText("Wave 1 · Initial")).toBeInTheDocument()
    expect(within(selection).getByText("Wave 2 · Child expansion")).toBeInTheDocument()
    expect(within(selection).getByText("Wave 3 · Fallback")).toBeInTheDocument()
    await user.click(within(selection).getByRole("button", { name: "Show community IDs for wave 1" }))
    expect(within(selection).getByText("A")).toBeInTheDocument()
    expect(within(selection).getAllByText("Selected")).toHaveLength(2)
    expect(within(selection).getByText("Passed threshold · not retained")).toBeInTheDocument()
    expect(within(selection).getByText("Below threshold")).toBeInTheDocument()

    await user.click(within(selection).getByRole("button", { name: "Expand rating details for community A" }))
    expect(within(selection).getByText("Repeat 1")).toBeInTheDocument()
    await user.click(within(selection).getByRole("button", { name: "View Rating Prompt" }))
    expect(screen.getByTestId("rating-prompt-A-0").textContent).toBe("RATING PROMPT A\n  exact")
    await user.click(within(selection).getByRole("button", { name: "Copy exact Community A · Repeat 1 Prompt" }))
    await expect(navigator.clipboard.readText()).resolves.toBe("RATING PROMPT A\n  exact")
    await user.click(within(selection).getByRole("button", { name: "View Raw Rating Response" }))
    expect(screen.getByTestId("rating-response-A-0").textContent).toBe("RAW RATING A")
    expect(screen.getByRole("article", { name: "Community Context" })).toHaveTextContent("0 community reports")
  })

  it("shows rating content as not captured in Metadata mode", async () => {
    const user = userEvent.setup()
    renderGlobal(dynamicEvents(false))
    const selection = screen.getByRole("article", { name: "Community Selection" })
    await user.click(within(selection).getByRole("button", { name: "Expand rating details for community A" }))
    await user.click(within(selection).getByRole("button", { name: "View Rating Prompt" }))
    expect(within(selection).getByText(/Rating prompt content was not captured/)).toBeInTheDocument()
    await user.click(within(selection).getByRole("button", { name: "View Raw Rating Response" }))
    expect(within(selection).getByText(/Raw rating response content was not captured/)).toBeInTheDocument()
  })

  it("bounds large Dynamic community lists with Show all and Show fewer", async () => {
    const user = userEvent.setup()
    const ratings = Array.from({ length: 21 }, (_, index) => ({ community_id: `community-${index}`, report_id: `report-${index}`, level: 0, selected_rating: 0, threshold_passed: false, selected: false }))
    renderGlobal([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "selection", { type: "dynamic_community_selection_started", initial_community_count: 21, threshold: 3, max_level: 2, keep_parent: false, use_summary: false, num_repeats: 1 }, "root"),
      envelope(3, "selection", { type: "dynamic_community_selection_completed", visited_count: 21, threshold_passed_count: 0, selected_count: 0, selected_community_ids: [], selected_report_ids: [], ratings }, "root"),
    ])
    const selection = screen.getByRole("article", { name: "Community Selection" })
    expect(within(selection).queryByText("Community community-20")).not.toBeInTheDocument()
    await user.click(within(selection).getByRole("button", { name: "Show all 21 communities" }))
    expect(within(selection).getByText("Community community-20")).toBeInTheDocument()
    await user.click(within(selection).getByRole("button", { name: "Show fewer communities" }))
    expect(within(selection).queryByText("Community community-20")).not.toBeInTheDocument()
  })

  it("expands batches and separates exact context, raw response, and parsed points", async () => {
    const user = userEvent.setup()
    renderGlobal()
    const community = screen.getByRole("article", { name: "Community Context" })
    await user.click(within(community).getByRole("button", { name: "Expand map batch 1" }))
    expect(within(community).getByText("report-a")).toBeInTheDocument()
    await user.click(within(community).getByRole("button", { name: "View Map Context" }))
    expect(screen.getByTestId("exact-map-context-0").textContent).toBe("MAP CONTEXT 0\n  preserve whitespace\n")
    await user.click(within(community).getByRole("button", { name: "Copy exact Map Batch 1 Context" }))
    await expect(navigator.clipboard.readText()).resolves.toBe("MAP CONTEXT 0\n  preserve whitespace\n")
    await user.click(within(community).getByRole("button", { name: "Collapse map batch 1" }))
    expect(within(community).queryByText("report-a")).not.toBeInTheDocument()

    const map = screen.getByRole("article", { name: "Map Analysis" })
    await user.click(within(map).getByRole("button", { name: "Expand Map analysis batch 1" }))
    expect(within(map).getByText("Completed · 2 points")).toBeInTheDocument()
    expect(within(map).getByText("PARSED POINT ANSWER")).toBeInTheDocument()
    expect(within(map).queryByText("RAW MAP RESPONSE 0")).not.toBeInTheDocument()
    await user.click(within(map).getByRole("button", { name: "View Raw Map Response" }))
    expect(screen.getByTestId("raw-map-response-0").textContent).toBe("RAW MAP RESPONSE 0")
    await user.click(within(map).getByRole("button", { name: "View Map Prompt" }))
    expect(screen.getByTestId("exact-map-prompt-0").textContent).toBe("MAP PROMPT 0")
  })

  it("shows Reduce decisions and exact backend Reduce Context without invented ranks", async () => {
    const user = userEvent.setup()
    renderGlobal()
    const reduce = screen.getByRole("article", { name: "Evidence Reduction" })

    expect(within(reduce).getAllByText("Included")).toHaveLength(2)
    expect(within(reduce).getAllByText("Non-positive")).toHaveLength(2)
    expect(within(reduce).getByText("Token budget")).toBeInTheDocument()
    expect(within(reduce).getByText("Token-budget excluded")).toBeInTheDocument()
    expect(within(reduce).queryByText(/Rank/)).not.toBeInTheDocument()
    await user.click(within(reduce).getByRole("button", { name: "View Reduce Context" }))
    expect(screen.getByTestId("exact-reduce-context").textContent).toBe("REDUCE CONTEXT\n  exact\n")
  })

  it("renders explicit unavailable states for metadata content", async () => {
    const user = userEvent.setup()
    renderGlobal(globalEvents(false))
    const community = screen.getByRole("article", { name: "Community Context" })
    await user.click(within(community).getByRole("button", { name: "Expand map batch 1" }))
    await user.click(within(community).getByRole("button", { name: "View Map Context" }))
    expect(within(community).getByText(/Map context content was not captured/)).toBeInTheDocument()

    const map = screen.getByRole("article", { name: "Map Analysis" })
    await user.click(within(map).getByRole("button", { name: "Expand Map analysis batch 1" }))
    expect(within(map).getAllByText(/Point answer was not captured/)).toHaveLength(2)
    await user.click(within(map).getByRole("button", { name: "View Map Prompt" }))
    expect(within(map).getByText(/Map prompt content was not captured/)).toBeInTheDocument()
    await user.click(within(map).getByRole("button", { name: "View Raw Map Response" }))
    expect(within(map).getByText(/Raw Map response content was not captured/)).toBeInTheDocument()

    const reduce = screen.getByRole("article", { name: "Evidence Reduction" })
    await user.click(within(reduce).getByRole("button", { name: "View Reduce Context" }))
    expect(within(reduce).getByText(/Reduce context content was not captured/)).toBeInTheDocument()

    const answer = screen.getByRole("article", { name: "Answer Generation" })
    await user.click(within(answer).getByRole("button", { name: "View Reduce Prompt" }))
    expect(within(answer).getByText(/Reduce prompt content was not captured/)).toBeInTheDocument()
    await user.click(within(answer).getByRole("button", { name: "View Raw Reduce Response" }))
    expect(within(answer).getByText(/Raw Reduce response content was not captured/)).toBeInTheDocument()
  })

  it("bounds large batch and point lists with Show all and Show fewer controls", async () => {
    const user = userEvent.setup()
    const events = [envelope(1, "root", { type: "query_started", method: "global" }), envelope(2, "context", { type: "global_context_built", batch_count: 7, report_count: 7 }, "root"), envelope(3, "map", { type: "global_map_started", batch_count: 7 }, "root")]
    for (let index = 0; index < 7; index += 1) events.push(envelope(index + 4, `batch-${index}`, { type: "global_map_batch_built", batch_index: index, report_count: 1, report_ids: [`report-${index}`], tokens_used: 10, token_budget: 20 }, "map"))
    events.push(envelope(18, "batch-0", { type: "llm_request_started", model_id: "map", prompt_tokens: 10 }, "map"))
    events.push(envelope(19, "batch-0", { type: "llm_request_completed", model_id: "map", input_tokens: 10, output_tokens: 2, elapsed_ms: 3 }, "map"))
    events.push(envelope(20, "batch-0", { type: "global_map_points_produced", batch_index: 0, points: Array.from({ length: 21 }, (_, index) => ({ batch_index: 0, point_index: index, score: index })) }, "map"))
    renderGlobal(events)

    const community = screen.getByRole("article", { name: "Community Context" })
    expect(within(community).queryByText("Batch 7")).not.toBeInTheDocument()
    await user.click(within(community).getByRole("button", { name: "Show all 7 batches" }))
    expect(within(community).getByText("Batch 7")).toBeInTheDocument()
    await user.click(within(community).getByRole("button", { name: "Show fewer batches" }))
    expect(within(community).queryByText("Batch 7")).not.toBeInTheDocument()

    const map = screen.getByRole("article", { name: "Map Analysis" })
    await user.click(within(map).getByRole("button", { name: "Expand Map analysis batch 1" }))
    expect(within(map).queryByText("Point 20")).not.toBeInTheDocument()
    await user.click(within(map).getByRole("button", { name: "Show all 21 points" }))
    expect(within(map).getByText("Point 20")).toBeInTheDocument()
    await user.click(within(map).getByRole("button", { name: "Show fewer points" }))
    expect(within(map).queryByText("Point 20")).not.toBeInTheDocument()
  })

  it("shows no-positive semantics without a fake Reduce LLM call", () => {
    renderGlobal([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
    ])

    expect(screen.getByText("Reduce skipped — no positive points")).toBeInTheDocument()
    expect(screen.getByText("No-data path selected. The Reduce LLM was skipped.")).toBeInTheDocument()
    expect(screen.queryByText(/answer returned/)).not.toBeInTheDocument()
    expect(screen.queryByText("Answer generated")).not.toBeInTheDocument()
  })

  it("only says the no-data answer was returned after RunCompleted", () => {
    renderGlobal([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
      envelope(4, "root", { type: "run_completed", elapsed_ms: 4 }),
    ])
    expect(screen.getByText("No-data answer returned. The Reduce LLM was not invoked.")).toBeInTheDocument()
  })

  it("never says the no-data answer was returned after RunFailed", () => {
    renderGlobal([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 1, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [{ batch_index: 0, point_index: 0, score: 0, selected: false, reason: "non_positive_score" }] }, "root"),
      envelope(3, "reduce", { type: "global_reduce_skipped", reason: "no_positive_points" }, "root"),
      envelope(4, "root", { type: "run_failed", error_kind: "query_completion", message: "failed" }),
    ])
    expect(screen.getByText("No-data path selected. The Reduce LLM was skipped.")).toBeInTheDocument()
    expect(screen.queryByText(/answer returned/)).not.toBeInTheDocument()
  })

  it("keeps span topology visible inside Technical details", async () => {
    const user = userEvent.setup()
    renderGlobal()
    const map = screen.getByRole("article", { name: "Map Analysis" })
    await user.click(within(map).getByRole("button", { name: /Technical details/ }))
    expect(within(map).getAllByText(/span batch-0 · parent map/).length).toBeGreaterThan(0)
  })

  it("shows future Global stages as pending until their facts are emitted", () => {
    renderGlobal([envelope(1, "root", { type: "query_started", method: "global" })])

    expect(screen.getByText("Waiting for community context")).toBeInTheDocument()
    expect(screen.getByText("Waiting for Map analysis")).toBeInTheDocument()
    expect(screen.getByText("Waiting for evidence reduction")).toBeInTheDocument()
    expect(screen.queryByText(/0 candidates/)).not.toBeInTheDocument()
    expect(screen.queryByText(/Reduce context content was not captured/)).not.toBeInTheDocument()
  })

  it("renders factual zero Reduce counts only after Reduce Context is built", async () => {
    const user = userEvent.setup()
    renderGlobal([
      envelope(1, "root", { type: "query_started", method: "global" }),
      envelope(2, "reduce", { type: "global_reduce_context_built", candidate_point_count: 0, positive_point_count: 0, selected_point_count: 0, token_budget: 100, tokens_used: 0, truncated: false, points: [] }, "root"),
    ])

    const reduce = screen.getByRole("article", { name: "Evidence Reduction" })
    expect(within(reduce).getByText(/0 candidates · 0 positive · 0 included/)).toBeInTheDocument()
    await user.click(within(reduce).getByRole("button", { name: "View Reduce Context" }))
    expect(within(reduce).getByText(/Reduce context content was not captured/)).toBeInTheDocument()
  })

  it("resets run-specific expanded batch state when switching methods and Runs", async () => {
    const user = userEvent.setup()
    const view = renderGlobal(globalEvents(), "global-a")
    const community = screen.getByRole("article", { name: "Community Context" })
    await user.click(within(community).getByRole("button", { name: "Expand map batch 1" }))
    expect(within(community).getByText("report-a")).toBeInTheDocument()

    view.rerender(<Timeline runId="local-b" envelopes={[envelope(1, "root", { type: "query_started", method: "local" }), envelope(2, "mapping", { type: "candidates_retrieved", candidates: [] })]} streamStatus="closed" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} />)
    expect(screen.getByRole("article", { name: "Entity Mapping" })).toBeInTheDocument()
    expect(screen.queryByRole("article", { name: "Community Context" })).not.toBeInTheDocument()

    view.rerender(<Timeline runId="global-c" envelopes={globalEvents()} streamStatus="closed" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} />)
    const freshCommunity = screen.getByRole("article", { name: "Community Context" })
    expect(within(freshCommunity).queryByText("report-a")).not.toBeInTheDocument()
  })
})
