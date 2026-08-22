import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"

afterEach(cleanup)

function envelope(sequence: number, spanId: string, event: ExplainabilityEventPayload, parentSpanId?: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "drift-run", timestamp: new Date(sequence * 10).toISOString(), span_id: spanId, ...(parentSpanId === undefined ? {} : { parent_span_id: parentSpanId }), event } }
}

function driftEvents(content = true): ExplainabilityEnvelope[] {
  const optional = <T extends Record<string, unknown>>(value: T): T | Record<string, never> => content ? value : {}
  return [
    envelope(1, "root", { type: "query_started", method: "drift", ...optional({ query: "ROOT" }) }),
    envelope(2, "hyde", { type: "drift_hyde_started", template_report_id: "stable-report-id", template_short_id: "R1", template_community_id: "community-1", template_index: 0, report_count: 1 }, "root"),
    envelope(3, "hyde", { type: "llm_request_started", model_id: "chat", prompt_tokens: 10, ...optional({ prompt: "HYDE EXACT" }) }, "root"),
    envelope(4, "hyde", { type: "llm_request_completed", model_id: "chat", input_tokens: 10, output_tokens: 2, elapsed_ms: 4, ...optional({ response: "HYDE RAW" }) }, "root"),
    envelope(5, "hyde", { type: "drift_hyde_completed", used_original_query: true }, "root"),
    envelope(6, "embedding", { type: "embedding_started", model_id: "embed", ...optional({ input: "ROOT" }) }, "root"),
    envelope(7, "embedding", { type: "embedding_completed", model_id: "embed", prompt_tokens: 1, dimensions: 2 }, "root"),
    envelope(8, "ranking", { type: "drift_reports_ranked", reports: [{ report_id: "report-2", short_id: "R2", community_id: "community-2", similarity: 0.4, rank: 1 }, { report_id: "report-1", short_id: "R1", community_id: "community-1", similarity: 0.9, rank: 2 }] }, "root"),
    envelope(9, "primer", { type: "drift_primer_started", fold_count: 2, ranked_report_count: 2 }, "root"),
    envelope(10, "fold-0", { type: "drift_primer_fold_started", fold_index: 0, fold_count: 2, report_ids: ["report-2", "report-1"] }, "primer"),
    envelope(11, "fold-1", { type: "drift_primer_fold_started", fold_index: 1, fold_count: 2, report_ids: [] }, "primer"),
    envelope(12, "fold-1", { type: "drift_primer_fold_completed", fold_index: 1, score: 40, follow_up_count: 0, ...optional({ intermediate_answer: "EMPTY FOLD ANSWER", follow_up_queries: [] }) }, "primer"),
    envelope(13, "fold-0", { type: "drift_primer_fold_completed", fold_index: 0, score: 80, follow_up_count: 2, ...optional({ intermediate_answer: "FOLD ANSWER", follow_up_queries: ["same", "same"] }) }, "primer"),
    envelope(14, "primer", { type: "drift_primer_completed", score: 77.7, root_action_id: 0, follow_up_count: 3, follow_up_action_ids: [1, 1, 2], ...optional({ answer: "PRIMER ANSWER", follow_up_queries: ["same", "same", "other"] }) }, "root"),
    envelope(15, "exploration", { type: "drift_exploration_started", max_depth: 2, selection_limit: 2, root_action_id: 0 }, "root"),
    envelope(16, "exploration", { type: "drift_depth_actions_selected", depth_index: 0, candidate_action_ids: [1, 2], selected_action_ids: [2, 1], selection_limit: 2 }, "root"),
    envelope(17, "attempt-2a", { type: "drift_action_attempt_started", depth_index: 0, action_id: 2, ...optional({ query: "other" }) }, "exploration"),
    envelope(18, "attempt-2a", { type: "drift_action_context_built", action_id: 2, ...optional({ context: "ACTION CONTEXT" }) }, "exploration"),
    envelope(19, "attempt-2a", { type: "llm_request_started", model_id: "chat", prompt_tokens: 20, ...optional({ prompt: "ACTION PROMPT" }) }, "exploration"),
    envelope(20, "attempt-2a", { type: "llm_request_completed", model_id: "chat", input_tokens: 20, output_tokens: 2, elapsed_ms: 3, ...optional({ response: "ACTION RAW" }) }, "exploration"),
    envelope(21, "attempt-2a", { type: "drift_action_attempt_completed", depth_index: 0, action_id: 2, answer_present: false, answer_non_empty: false, follow_up_count: 1, target_action_ids: [3], ...optional({ follow_up_queries: ["shared"] }) }, "exploration"),
    envelope(22, "attempt-1", { type: "drift_action_attempt_started", depth_index: 0, action_id: 1, ...optional({ query: "same" }) }, "exploration"),
    envelope(23, "attempt-1", { type: "drift_action_attempt_completed", depth_index: 0, action_id: 1, answer_present: true, answer_non_empty: false, score: 50, follow_up_count: 1, target_action_ids: [3], ...optional({ answer: "", follow_up_queries: ["shared"] }) }, "exploration"),
    envelope(24, "exploration", { type: "drift_depth_actions_selected", depth_index: 1, candidate_action_ids: [2, 3], selected_action_ids: [2], selection_limit: 2 }, "root"),
    envelope(25, "attempt-2b", { type: "drift_action_attempt_started", depth_index: 1, action_id: 2, ...optional({ query: "other" }) }, "exploration"),
    envelope(26, "attempt-2b", { type: "drift_action_attempt_completed", depth_index: 1, action_id: 2, answer_present: true, answer_non_empty: true, score: 70, follow_up_count: 0, target_action_ids: [], ...optional({ answer: "   ", follow_up_queries: [] }) }, "exploration"),
    envelope(27, "reduce", { type: "drift_reduce_context_built", node_count: 4, edge_count: 5, included_answer_count: 2, included_action_ids: [0, 2], ...optional({ state_context: "{\"exact\":  true}", reduce_context: "['PRIMER ANSWER', '   ']" }) }, "root"),
    envelope(28, "reduce", { type: "llm_request_started", model_id: "chat", prompt_tokens: 30, ...optional({ prompt: "REDUCE EXACT" }) }, "root"),
    envelope(29, "reduce", { type: "llm_request_completed", model_id: "chat", input_tokens: 30, output_tokens: 4, elapsed_ms: 5, ...optional({ response: "FINAL RAW" }) }, "root"),
    envelope(30, "root", { type: "run_completed", elapsed_ms: 100 }),
  ]
}

function renderDrift(events = driftEvents()): ReturnType<typeof render> {
  return render(<Timeline runId="drift-run" envelopes={events} streamStatus="closed" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} />)
}

describe("DRIFT Timeline", () => {
  it("renders only Primer & Ranking, Exploration, and Final Synthesis", () => {
    renderDrift()
    expect(screen.getAllByRole("article").map((article) => article.getAttribute("aria-label")).filter(Boolean)).toEqual(["Primer & Ranking", "Exploration", "Final Synthesis"])
    expect(screen.getByText("2 reports ranked · 2 primer folds · 3 follow-up queries · Primer score 77.7")).toBeInTheDocument()
  })

  it("shows backend report order, empty folds, and aggregate without reranking or recomputing", async () => {
    const user = userEvent.setup()
    renderDrift()
    const primer = screen.getByRole("article", { name: "Primer & Ranking" })
    const reports = within(primer).getByRole("region", { name: "Ranked Reports" })
    expect(within(reports).getAllByText(/Report R[12]/).map((node) => node.textContent)).toEqual(["Report R2", "Report R1"])
    expect(within(primer).getByText("77.7")).toBeInTheDocument()
    const emptyFold = within(primer).getByRole("button", { name: "Expand Primer Fold 2" })
    expect(emptyFold).toHaveAttribute("aria-expanded", "false")
    await user.click(emptyFold)
    expect(within(primer).getByText("0 reports")).toBeInTheDocument()
  })

  it("labels random selection and preserves duplicate edges in the Action Graph", () => {
    renderDrift()
    const exploration = screen.getByRole("article", { name: "Exploration" })
    expect(within(exploration).getAllByText("Randomly selected from incomplete actions")).toHaveLength(2)
    expect(within(exploration).queryByText(/Best actions/)).not.toBeInTheDocument()
    expect(within(exploration).getAllByText("#0 → #1")).toHaveLength(2)
    expect(within(exploration).getByText("Follow-up reasoning graph. Nodes are exact query identities; duplicate edges and multiple parents are preserved.")).toBeInTheDocument()
  })

  it("groups repeated attempts and distinguishes incomplete, empty, whitespace, and not explored", async () => {
    const user = userEvent.setup()
    renderDrift()
    const exploration = screen.getByRole("article", { name: "Exploration" })
    await user.click(within(exploration).getByRole("button", { name: "Expand Action 2" }))
    expect(within(exploration).getByText("Attempt 1 · Depth 1")).toBeInTheDocument()
    expect(within(exploration).getByText("Attempt 2 · Depth 2")).toBeInTheDocument()
    expect(within(exploration).getByText("Remains incomplete")).toBeInTheDocument()
    expect(within(exploration).getByText("Completed")).toBeInTheDocument()
    expect(within(exploration).getByRole("button", { name: "Expand Action 1" }).parentElement).toHaveTextContent("Completed · empty answer · 1 attempts")
    expect(within(exploration).getByRole("button", { name: "Expand Action 3" }).parentElement).toHaveTextContent("Not explored")
    expect(within(exploration).queryByText(/Failed/)).not.toBeInTheDocument()
  })

  it("copies exact backend state and Python Reduce context without rebuilding", async () => {
    const user = userEvent.setup()
    renderDrift()
    await user.click(screen.getByRole("button", { name: "View DRIFT State" }))
    expect(screen.getByTestId("exact-drift-state").textContent).toBe("{\"exact\":  true}")
    await user.click(screen.getByRole("button", { name: "Copy exact DRIFT state" }))
    await expect(navigator.clipboard.readText()).resolves.toBe("{\"exact\":  true}")
    await user.click(screen.getByRole("button", { name: "View Reduce Context" }))
    expect(screen.getByTestId("exact-drift-reduce-context").textContent).toBe("['PRIMER ANSWER', '   ']")
    const reduceSelection = screen.getByRole("region", { name: "Reduce selection" })
    expect(within(reduceSelection).getByText("#0")).toBeInTheDocument()
    expect(within(reduceSelection).getByText("#2")).toBeInTheDocument()
  })

  it("shows content as not captured in Metadata mode", async () => {
    const user = userEvent.setup()
    renderDrift(driftEvents(false))
    await user.click(screen.getByRole("button", { name: "View HyDE Prompt" }))
    expect(screen.getByText(/HyDE prompt was not captured/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "View DRIFT State" }))
    expect(screen.getByText(/DRIFT state content was not captured/)).toBeInTheDocument()
  })

  it("never offers Knowledge Graph focus from DRIFT semantic evidence", () => {
    renderDrift()
    expect(screen.queryByRole("button", { name: "Focus in graph" })).not.toBeInTheDocument()
    expect(screen.queryByText("Exploration Tree")).not.toBeInTheDocument()
  })

  it("switches Local, Global, Basic, DRIFT, and Local Runs without residual semantic state", () => {
    const props = { streamStatus: "closed" as const, onFocusGraph: vi.fn(), onInspectCandidate: vi.fn() }
    const local = [envelope(1, "local-root", { type: "query_started", method: "local" }), envelope(2, "local-map", { type: "candidates_retrieved", record_type: "entity", candidates: [] }, "local-root")]
    const global = [envelope(1, "global-root", { type: "query_started", method: "global" })]
    const basic = [envelope(1, "basic-root", { type: "query_started", method: "basic" })]
    const view = render(<Timeline {...props} runId="local-a" envelopes={local} />)
    expect(screen.getByRole("article", { name: "Entity Mapping" })).toBeInTheDocument()
    view.rerender(<Timeline {...props} runId="global-b" envelopes={global} />)
    expect(screen.getByRole("article", { name: "Community Context" })).toBeInTheDocument()
    expect(screen.queryByRole("article", { name: "Entity Mapping" })).not.toBeInTheDocument()
    view.rerender(<Timeline {...props} runId="basic-c" envelopes={basic} />)
    expect(screen.getByRole("article", { name: "Text Retrieval" })).toBeInTheDocument()
    expect(screen.queryByRole("article", { name: "Community Context" })).not.toBeInTheDocument()
    view.rerender(<Timeline {...props} runId="drift-d" envelopes={driftEvents()} />)
    expect(screen.getByRole("article", { name: "Exploration" })).toBeInTheDocument()
    expect(screen.queryByRole("article", { name: "Text Retrieval" })).not.toBeInTheDocument()
    view.rerender(<Timeline {...props} runId="local-e" envelopes={local} />)
    expect(screen.getByRole("article", { name: "Entity Mapping" })).toBeInTheDocument()
    expect(screen.queryByRole("article", { name: "Exploration" })).not.toBeInTheDocument()
  })
})
