import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityCandidate, ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"

afterEach(cleanup)

function envelope(sequence: number, spanId: string, event: ExplainabilityEventPayload, parentSpanId?: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "basic-run", timestamp: new Date(sequence * 10).toISOString(), span_id: spanId, ...(parentSpanId === undefined ? {} : { parent_span_id: parentSpanId }), event } }
}

function candidate(id: string, rank: number, selected = false, reason = "ann_result"): ExplainabilityCandidate {
  return { id, short_id: id.toLowerCase(), record_type: "text_unit", score: 1 - rank / 10, rank, selected, reason }
}

function basicEvents(content = true): ExplainabilityEnvelope[] {
  return [
    envelope(1, "root", { type: "query_started", method: "basic" }),
    envelope(2, "embedding", { type: "embedding_started", model_id: "embed", ...(content ? { input: "QUERY" } : {}) }, "root"),
    envelope(3, "embedding", { type: "embedding_completed", model_id: "embed", prompt_tokens: 2, dimensions: 2 }, "root"),
    envelope(4, "retrieval", { type: "candidates_retrieved", record_type: "text_unit", candidates: [candidate("C", 1), candidate("A", 2), candidate("B", 3)] }, "root"),
    envelope(5, "context", { type: "context_budget_allocated", total_token_budget: 20, sections: [{ section: "sources", token_budget: 20 }] }, "root"),
    envelope(6, "retrieval", { type: "candidates_filtered", record_type: "text_unit", candidates: [candidate("A", 2, true), candidate("B", 3, false, "token_budget"), candidate("C", 1, false, "token_budget")] }, "root"),
    envelope(7, "context", { type: "context_section_built", section: { section: "sources", token_budget: 20, tokens_used: 12, candidate_count: 3, selected_count: 1, truncated: true, selected_record_ids: ["A"] } }, "root"),
    envelope(8, "context", { type: "context_completed", tokens_used: 12, ...(content ? { context: "id|text\nA|exact  context\n" } : {}) }, "root"),
    envelope(9, "llm", { type: "llm_request_started", model_id: "chat", prompt_tokens: 30, ...(content ? { prompt: "BASIC PROMPT\n  exact" } : {}) }, "root"),
    envelope(10, "llm", { type: "llm_request_completed", model_id: "chat", input_tokens: 30, output_tokens: 4, elapsed_ms: 25, ...(content ? { response: "RAW BASIC RESPONSE" } : {}) }, "root"),
    envelope(11, "root", { type: "run_completed", elapsed_ms: 100 }),
  ]
}

function renderBasic(events = basicEvents()): ReturnType<typeof render> {
  return render(<Timeline runId="basic-run" envelopes={events} streamStatus="closed" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} />)
}

describe("Basic Timeline", () => {
  it("renders three semantic steps with ANN and effective orders kept distinct", () => {
    renderBasic()
    expect(screen.getAllByRole("article").map((article) => article.getAttribute("aria-label")).filter(Boolean)).toEqual(["Text Retrieval", "Context Assembly", "Answer Generation"])
    const retrieval = screen.getByRole("article", { name: "Text Retrieval" })
    const context = screen.getByRole("article", { name: "Context Assembly" })
    expect(within(retrieval).getAllByText(/Text Unit/).map((node) => node.textContent)).toEqual(["Text Unit c", "Text Unit a", "Text Unit b"])
    expect(within(context).getAllByText(/Text Unit/).map((node) => node.textContent)).toEqual(["Text Unit a", "Text Unit b", "Text Unit c"])
    expect(within(context).getByText(/preserves source-table order/)).toBeInTheDocument()
    expect(within(context).getAllByText("Not included after token-budget stop")).toHaveLength(2)
  })

  it("opens and copies exact context, prompt, and raw response without rebuilding content", async () => {
    const user = userEvent.setup()
    renderBasic()
    await user.click(screen.getByRole("button", { name: "View Basic Context" }))
    expect(screen.getByTestId("exact-basic-context").textContent).toBe("id|text\nA|exact  context\n")
    await user.click(screen.getByRole("button", { name: "Copy exact Basic context" }))
    await expect(navigator.clipboard.readText()).resolves.toBe("id|text\nA|exact  context\n")
    await user.click(screen.getByRole("button", { name: "View Basic Prompt" }))
    expect(screen.getByTestId("exact-basic-prompt").textContent).toBe("BASIC PROMPT\n  exact")
    await user.click(screen.getByRole("button", { name: "View Raw Basic Response" }))
    expect(screen.getByTestId("raw-basic-response").textContent).toBe("RAW BASIC RESPONSE")
  })

  it("shows metadata content as not captured rather than empty", async () => {
    const user = userEvent.setup()
    renderBasic(basicEvents(false))
    await user.click(screen.getByRole("button", { name: "View Basic Context" }))
    expect(screen.getByText(/Basic context content was not captured/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "View Basic Prompt" }))
    expect(screen.getByText(/Basic prompt content was not captured/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "View Raw Basic Response" }))
    expect(screen.getByText(/Raw Basic response content was not captured/)).toBeInTheDocument()
  })

  it("shows an intentional empty-query skip instead of zero ANN results", () => {
    renderBasic([
      envelope(1, "root", { type: "query_started", method: "basic" }),
      envelope(2, "retrieval", { type: "basic_retrieval_skipped", reason: "empty_query" }, "root"),
    ])
    const retrieval = screen.getByRole("article", { name: "Text Retrieval" })
    expect(within(retrieval).getByText("Retrieval skipped — empty query")).toBeInTheDocument()
    expect(within(retrieval).queryByText(/0 text units retrieved/)).not.toBeInTheDocument()
  })

  it("bounds large candidate lists with Show all and Show fewer", async () => {
    const user = userEvent.setup()
    const candidates = Array.from({ length: 21 }, (_, index) => candidate(`ID-${index}`, index + 1))
    renderBasic([
      envelope(1, "root", { type: "query_started", method: "basic" }),
      envelope(2, "retrieval", { type: "candidates_retrieved", record_type: "text_unit", candidates }, "root"),
    ])
    const retrieval = screen.getByRole("article", { name: "Text Retrieval" })
    expect(within(retrieval).queryByText("Text Unit id-20")).not.toBeInTheDocument()
    const showAll = within(retrieval).getByRole("button", { name: "Show all 21 Basic text units" })
    expect(showAll).toHaveAttribute("aria-expanded", "false")
    await user.click(showAll)
    expect(within(retrieval).getByText("Text Unit id-20")).toBeInTheDocument()
    const showFewer = within(retrieval).getByRole("button", { name: "Show fewer Basic text units" })
    expect(showFewer).toHaveAttribute("aria-expanded", "true")
    await user.click(showFewer)
    expect(within(retrieval).queryByText("Text Unit id-20")).not.toBeInTheDocument()
  })

  it("keeps raw lifecycle events available in Diagnostics and technical details", async () => {
    const user = userEvent.setup()
    renderBasic()
    const retrieval = screen.getByRole("article", { name: "Text Retrieval" })
    await user.click(within(retrieval).getByRole("button", { name: /Technical details/ }))
    expect(within(retrieval).getByText("Embedding started")).toBeInTheDocument()
    await user.click(screen.getByText(/Diagnostics \/ Raw events/))
    expect(screen.getByText("Query started")).toBeInTheDocument()
    expect(screen.getByText("Run completed")).toBeInTheDocument()
  })
})
