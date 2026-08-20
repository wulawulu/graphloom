import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"

afterEach(cleanup)

function envelope(event: ExplainabilityEventPayload, sequence = 4): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run", timestamp: "2026-08-09T00:00:00Z", span_id: "span", event } }
}

function renderTimeline(events: ExplainabilityEventPayload[], onInspectCandidate = vi.fn(), onFocusGraph = vi.fn()): ReturnType<typeof render> {
  return render(<Timeline runId="run" streamStatus="open" onFocusGraph={onFocusGraph} onInspectCandidate={onInspectCandidate} envelopes={events.map((event, index) => envelope(event, index + 1))} />)
}

async function openTechnicalDetails(user: ReturnType<typeof userEvent.setup>, stepName: string): Promise<void> {
  const step = screen.getByRole("article", { name: stepName })
  await user.click(within(step).getByRole("button", { name: /Technical details/ }))
}

describe("Timeline", () => {
  it("renders the empty state and keeps forward-compatible events in diagnostics", async () => {
    const user = userEvent.setup()
    const { rerender } = render(<Timeline runId={null} envelopes={[]} streamStatus="idle" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} />)
    expect(screen.getByText("No Run selected")).toBeInTheDocument()
    expect(screen.getByText("Choose a historical Run or submit a new Query.")).toBeInTheDocument()
    expect(screen.queryByText(/Local Query/)).not.toBeInTheDocument()
    rerender(<Timeline runId="run" streamStatus="open" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} envelopes={[envelope({ type: "future_graphloom_event", foo: "bar" })]} />)
    await user.click(screen.getByText(/Diagnostics \/ Raw events/))
    expect(screen.getByText("future graphloom event")).toBeInTheDocument()
    expect(screen.getByText("#4")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Details" }))
    expect(screen.getByText("Event details")).toBeInTheDocument()
    expect(screen.getByText("Developer data")).toBeInTheDocument()
  })

  it("presents four semantic decisions and moves lifecycle events behind disclosures", async () => {
    const user = userEvent.setup()
    renderTimeline([
      { type: "run_started" },
      { type: "query_started" },
      { type: "embedding_started", model_id: "bge-m3" },
      { type: "embedding_completed", model_id: "bge-m3", prompt_tokens: 5, dimensions: 1024 },
      { type: "candidates_retrieved", record_type: "entity", candidates: [] },
      { type: "relationships_selected", relationships: [] },
      { type: "context_section_built", section: { section: "entities", token_budget: 400, tokens_used: 20, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: ["entity-1"] } },
      { type: "llm_request_started", model_id: "model-x", prompt_tokens: 20 },
      { type: "llm_request_completed", model_id: "model-x", input_tokens: 20, output_tokens: 4, elapsed_ms: 10 },
      { type: "run_completed" },
    ])

    expect(screen.getAllByRole("article").map((article) => article.getAttribute("aria-label")).filter((label) => label !== null)).toEqual(["Entity Mapping", "Graph Expansion", "Context Assembly", "Answer Generation"])
    expect(screen.queryByText("Embedding started")).not.toBeInTheDocument()
    expect(screen.queryByText("Embedding completed")).not.toBeInTheDocument()
    await openTechnicalDetails(user, "Entity Mapping")
    expect(screen.getByText("Embedding started")).toBeInTheDocument()
    expect(screen.getByText("Embedding completed")).toBeInTheDocument()
    expect(screen.getByText("Run started")).not.toBeVisible()
    await user.click(screen.getByText(/Diagnostics \/ Raw events/))
    expect(screen.getByText("Run started")).toBeInTheDocument()
    expect(screen.getByText("Run completed")).toBeInTheDocument()
  })

  it("inspects stable graph candidates and leaves unsupported records read-only", async () => {
    const user = userEvent.setup()
    const onInspectCandidate = vi.fn()
    renderTimeline([
      { type: "entities_selected", entities: [{ id: "entity-1", title: "Alice", record_type: "entity", score: 0.91, rank: 1, selected: true }] },
      { type: "relationships_selected", relationships: [{ id: "relationship-1", title: "Alice knows Bob", record_type: "relationship", selected: true }] },
      { type: "community_reports_selected", community_reports: [{ id: "report-1", title: "Report one", record_type: "community_report", selected: true }] },
    ], onInspectCandidate)

    await user.click(screen.getByRole("button", { name: "Inspect entity Alice" }))
    expect(onInspectCandidate).toHaveBeenCalledWith(expect.objectContaining({ stableId: "entity-1", recordType: "entity" }))
    await user.click(screen.getByRole("button", { name: "Inspect relationship Alice knows Bob" }))
    expect(onInspectCandidate).toHaveBeenCalledWith(expect.objectContaining({ stableId: "relationship-1", recordType: "relationship" }))
    expect(screen.getByText("Report one")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /Inspect community_report/ })).not.toBeInTheDocument()
  })

  it("shows selected and final-context decisions as distinct states", () => {
    renderTimeline([
      { type: "entities_selected", entities: [{ id: "entity-in", title: "Included entity", record_type: "entity", selected: true }, { id: "entity-out", title: "Truncated entity", record_type: "entity", selected: true }, { id: "entity-rejected", title: "Rejected entity", record_type: "entity", selected: false }] },
      { type: "context_section_built", section: { section: "entities", token_budget: 400, tokens_used: 20, candidate_count: 3, selected_count: 1, truncated: true, selected_record_ids: ["entity-in"] } },
    ])

    expect(screen.getByText("Included entity").closest("div.grid")).toHaveTextContent("Included")
    expect(screen.getByText("Truncated entity").closest("div.grid")).toHaveTextContent("Not in final context")
    expect(screen.getByText("Rejected entity").closest("div.grid")).toHaveTextContent("Excluded")
  })

  it("updates live candidates from Retrieved to explicit selection decisions", () => {
    const retrieved = envelope({ type: "candidates_retrieved", candidates: [{ id: "entity-1", title: "Alice", record_type: "entity", selected: false }, { id: "entity-2", title: "Bob", record_type: "entity", selected: false }] }, 1)
    const common = { runId: "run", streamStatus: "open" as const, onFocusGraph: vi.fn(), onInspectCandidate: vi.fn() }
    const { rerender } = render(<Timeline {...common} envelopes={[retrieved]} />)

    expect(screen.getByText("Alice").closest("div.grid")).toHaveTextContent("Retrieved")
    expect(screen.getByText("Bob").closest("div.grid")).toHaveTextContent("Retrieved")
    expect(screen.getByText("2 retrieved · 0 selected · 0 excluded · 2 pending")).toBeInTheDocument()

    const filtered = envelope({ type: "candidates_filtered", candidates: [{ id: "entity-1", title: "Alice", record_type: "entity", selected: true, reason: "ann_result" }, { id: "entity-2", title: "Bob", record_type: "entity", selected: false, reason: "explicitly_excluded" }] }, 2)
    rerender(<Timeline {...common} envelopes={[retrieved, filtered]} />)

    expect(screen.getByText("Alice").closest("div.grid")).toHaveTextContent("Selected · context unknown")
    expect(screen.getByText("Bob").closest("div.grid")).toHaveTextContent("Excluded")
    expect(screen.getByText("2 retrieved · 1 selected · 1 excluded")).toBeInTheDocument()
  })

  it("preserves typed candidate details and bounded raw tables", async () => {
    const user = userEvent.setup()
    const candidates = Array.from({ length: 101 }, (_, index) => ({ id: `entity-${index + 1}`, title: `Candidate ${index + 1}`, record_type: "entity", selected: false }))
    renderTimeline([{ type: "candidates_retrieved", record_type: "entity", candidates }])
    expect(screen.queryByText("Candidate 101")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Show all 101 records" }))
    expect(screen.getByText("Candidate 101")).toBeInTheDocument()
    await openTechnicalDetails(user, "Entity Mapping")
    await user.click(screen.getByRole("button", { name: "Details" }))
    expect(screen.getByText("Showing 100 of 101")).toBeInTheDocument()
    expect(screen.getByText("Developer data")).toBeInTheDocument()
  })

  it("preserves context and LLM technical details", async () => {
    const user = userEvent.setup()
    renderTimeline([
      { type: "context_budget_allocated", total_token_budget: 1000, sections: [{ section: "entities", token_budget: 400 }] },
      { type: "context_completed", tokens_used: 700 },
      { type: "llm_request_completed", model_id: "model-x", input_tokens: 700, output_tokens: 40, elapsed_ms: 25, response: "Explainability response" },
    ])
    await openTechnicalDetails(user, "Context Assembly")
    const contextDetails = screen.getAllByRole("button", { name: "Details" })
    await user.click(contextDetails[0]!)
    expect(screen.getByText("Total token budget")).toBeInTheDocument()
    await user.click(contextDetails[1]!)
    expect(screen.getByText("Content hidden by explainability mode.")).toBeInTheDocument()
    await openTechnicalDetails(user, "Answer Generation")
    await user.click(screen.getAllByRole("button", { name: "Details" })[2]!)
    expect(screen.getByText("Explainability response")).toBeInTheDocument()
  })

  it("shows, previews, and copies the exact captured LLM context without changing whitespace", async () => {
    const user = userEvent.setup()
    const exact = "Reports\n[]\n\nEntities\n  id,title\n  1,Alice\n"
    renderTimeline([
      { type: "context_section_built", section: { section: "community_reports", name: "Community Reports", token_budget: 1_800, tokens_used: 1, candidate_count: 20, selected_count: 0, truncated: true, selected_record_ids: [] } },
      { type: "context_completed", tokens_used: 17, context: exact },
    ])

    expect(screen.getByText("0 / 20 included")).toBeInTheDocument()
    expect(screen.getByText("1 / 1,800 tokens · empty section · truncated")).toHaveAttribute("title", expect.stringContaining("literal final section text"))
    await user.click(screen.getByRole("button", { name: "View LLM Context" }))
    expect(screen.getByTestId("exact-llm-context")).toHaveTextContent(exact, { normalizeWhitespace: false })
    expect(screen.getByTestId("exact-llm-context").textContent).toBe(exact)
    await user.click(screen.getByRole("button", { name: "Copy exact LLM context" }))
    await expect(navigator.clipboard.readText()).resolves.toBe(exact)
    await user.click(screen.getByRole("tab", { name: "Preview" }))
    expect(screen.getByText(/Reports\s*\[\]/)).toBeInTheDocument()
    await user.click(screen.getByRole("tab", { name: "Exact input" }))
    expect(screen.getByTestId("exact-llm-context").textContent).toBe(exact)
  })

  it("does not fabricate LLM context for metadata-only runs", async () => {
    const user = userEvent.setup()
    renderTimeline([{ type: "context_completed", tokens_used: 1 }])
    await user.click(screen.getByRole("button", { name: "View LLM Context" }))

    expect(screen.getByText(/LLM context content was not captured/)).toBeInTheDocument()
    expect(screen.queryByTestId("exact-llm-context")).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Copy exact LLM context" })).not.toBeInTheDocument()
  })

  it("keeps long decision records inside a bounded two-row grid and preserves inspection", async () => {
    const user = userEvent.setup()
    const onInspect = vi.fn()
    const title = "A very long entity title that must remain readable without widening the Explainability panel"
    const id = "ca426c13-1234-5678-90ab-0123459ab03f"
    renderTimeline([{ type: "entities_selected", entities: [{ id, short_id: "7", title, record_type: "entity", score: 0.8231, selected: false }] }], onInspect)

    const button = screen.getByRole("button", { name: `Inspect entity ${title}` })
    const row = button.closest("div.grid")
    expect(row).toHaveClass("min-w-0", "overflow-hidden", "grid-cols-[auto_minmax(0,1fr)_auto]")
    expect(button).toHaveClass("truncate")
    expect(screen.getByTitle(id)).toHaveClass("truncate")
    expect(screen.getByText("Excluded")).toHaveClass("shrink-0", "truncate")
    expect(screen.getByLabelText("Score 0.8231")).toHaveClass("shrink-0")
    expect(screen.getByText("7 · ca426c13…459ab03f")).toBeInTheDocument()
    await user.click(button)
    expect(onInspect).toHaveBeenCalledWith(expect.objectContaining({ stableId: id }))
  })

  it("keeps Focus in graph explicit and excludes rejected-only mappings", async () => {
    const user = userEvent.setup()
    const onFocusGraph = vi.fn()
    const selected = { type: "entities_selected", entities: [{ id: "entity-1", title: "Alice", record_type: "entity", selected: true }] }
    const { rerender } = renderTimeline([selected], vi.fn(), onFocusGraph)
    await user.click(screen.getByRole("button", { name: "Focus in graph" }))
    expect(onFocusGraph).toHaveBeenCalledWith(expect.objectContaining({ record: expect.objectContaining({ event: selected }) }))

    rerender(<Timeline runId="run" streamStatus="open" onFocusGraph={onFocusGraph} onInspectCandidate={vi.fn()} envelopes={[envelope({ type: "entities_selected", entities: [{ id: "entity-rejected", title: "Rejected", record_type: "entity", selected: false }] })]} />)
    expect(screen.queryByRole("button", { name: "Focus in graph" })).not.toBeInTheDocument()
  })
})
