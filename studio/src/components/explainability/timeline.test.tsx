import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"

afterEach(cleanup)

function envelope(event: ExplainabilityEventPayload): ExplainabilityEnvelope {
  return { schema_version: 1, sequence: 4, record: { run_id: "run", timestamp: "2026-08-09T00:00:00Z", span_id: "span", event } }
}

function renderEvent(event: ExplainabilityEventPayload): ReturnType<typeof render> {
  return render(<Timeline runId="run" streamStatus="open" onFocusGraph={vi.fn()} envelopes={[envelope(event)]} />)
}

async function openDetails(user: ReturnType<typeof userEvent.setup>): Promise<void> {
  await user.click(screen.getByRole("button", { name: "Details" }))
}

describe("Timeline", () => {
  it("renders the empty state and a forward-compatible unknown event", async () => {
    const user = userEvent.setup()
    const { rerender } = render(<Timeline runId={null} envelopes={[]} streamStatus="idle" onFocusGraph={vi.fn()} />)
    expect(screen.getByText("No Run selected")).toBeInTheDocument()
    rerender(<Timeline runId="run" streamStatus="open" onFocusGraph={vi.fn()} envelopes={[envelope({ type: "future_graphloom_event", foo: "bar" })]} />)
    expect(screen.getByText("future graphloom event")).toBeInTheDocument()
    expect(screen.getByText("#4")).toBeInTheDocument()
    await openDetails(user)
    expect(screen.getByText("Event details")).toBeInTheDocument()
    expect(screen.getByText("Developer data")).toBeInTheDocument()
  })

  it("renders candidate events as a selected-aware table instead of a JSON primary view", async () => {
    const user = userEvent.setup()
    renderEvent({ type: "entities_selected", entities: [{ id: "entity-1", title: "Alice", record_type: "entity", score: 0.91, rank: 1, selected: true, reason: "ann_result" }, { id: "entity-2", title: "Bob", record_type: "entity", selected: false, reason: "token_budget" }] })
    await openDetails(user)

    expect(screen.getByRole("columnheader", { name: "Rank" })).toBeInTheDocument()
    expect(screen.getByText("Selected 1 / Candidates 2")).toBeInTheDocument()
    expect(screen.getByText("Alice").closest("tr")).toHaveAttribute("data-selected", "true")
    expect(screen.getByText("Bob").closest("tr")).toHaveAttribute("data-selected", "false")
    expect(screen.getByRole("button", { name: "Focus all in graph" })).toBeInTheDocument()
    expect(screen.getByText("Developer data")).toBeInTheDocument()
    expect(screen.getByText("Raw JSON")).toBeInTheDocument()
  })

  it("reveals candidates in bounded batches of 100", async () => {
    const user = userEvent.setup()
    const candidates = Array.from({ length: 101 }, (_, index) => ({ id: `entity-${index + 1}`, title: `Candidate ${index + 1}`, record_type: "entity", selected: false }))
    renderEvent({ type: "candidates_retrieved", record_type: "entity", candidates })
    await openDetails(user)
    expect(screen.getByText("Showing 100 of 101")).toBeInTheDocument()
    expect(screen.queryByText("Candidate 101")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Show 100 more" }))
    expect(screen.getByText("Candidate 101")).toBeInTheDocument()
  })

  it("renders context budgets and metadata-hidden context safely", async () => {
    const user = userEvent.setup()
    const { unmount } = renderEvent({ type: "context_budget_allocated", total_token_budget: 1000, sections: [{ section: "entities", token_budget: 400 }, { section: "sources", token_budget: 0 }] })
    await openDetails(user)
    expect(screen.getByText("Total token budget")).toBeInTheDocument()
    expect(screen.getByText("entities")).toBeInTheDocument()
    unmount()

    renderEvent({ type: "context_completed", tokens_used: 700 })
    await openDetails(user)
    expect(screen.getByText("Content hidden by explainability mode.")).toBeInTheDocument()
  })

  it("renders typed LLM usage and disclosed response", async () => {
    const user = userEvent.setup()
    renderEvent({ type: "llm_request_completed", model_id: "model-x", input_tokens: 700, output_tokens: 40, elapsed_ms: 25, response: "Explainability response" })
    await openDetails(user)
    expect(screen.getByText("model-x")).toBeInTheDocument()
    expect(screen.getByText("700")).toBeInTheDocument()
    expect(screen.getByText("40")).toBeInTheDocument()
    expect(screen.getByText("Response")).toBeInTheDocument()
    expect(screen.getByText("Explainability response")).toBeInTheDocument()
  })

  it("renders graph expansion seeds and a focused action", async () => {
    const user = userEvent.setup()
    renderEvent({ type: "graph_expansion_started", seed_entity_ids: ["entity-a", "entity-b"] })
    await openDetails(user)
    expect(screen.getByText("entity-a")).toBeInTheDocument()
    expect(screen.getByText("entity-b")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Focus expansion in graph" })).toBeInTheDocument()
  })
})
