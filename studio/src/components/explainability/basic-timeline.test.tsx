import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { resolveTextUnits } from "@/api/client"
import type { ExplainabilityCandidate, ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { Timeline } from "@/components/explainability/timeline"
import { TextUnitEvidenceProvider } from "@/contexts/text-unit-evidence-provider"
import { setStudioLocale } from "@/i18n"

vi.mock("@/api/client", () => ({ resolveTextUnits: vi.fn() }))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

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

function renderBasicWithEvidence(events = basicEvents()): ReturnType<typeof render> {
  return render(<TextUnitEvidenceProvider><Timeline runId="basic-run" envelopes={events} streamStatus="closed" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} /></TextUnitEvidenceProvider>)
}

describe("Basic Timeline", () => {
  it("uses short IDs in retrieval and context while keeping stable UUIDs out of normal presentation", () => {
    const stableId = "71d89c81-1234-5678-90ab-12345678e942"
    const item = { id: stableId, short_id: "184", record_type: "text_unit", score: 0.8123, rank: 1, selected: true }
    renderBasic([
      envelope(1, "root", { type: "query_started", method: "basic" }),
      envelope(2, "retrieval", { type: "candidates_retrieved", record_type: "text_unit", candidates: [item] }, "root"),
      envelope(3, "retrieval", { type: "candidates_filtered", record_type: "text_unit", candidates: [item] }, "root"),
      envelope(4, "context", { type: "context_budget_allocated", total_token_budget: 20, sections: [{ section: "sources", token_budget: 20 }] }, "root"),
      envelope(5, "context", { type: "context_section_built", section: { section: "sources", token_budget: 20, tokens_used: 10, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: [stableId] } }, "root"),
    ])
    expect(screen.getAllByText("Text Unit 184")).toHaveLength(2)
    expect(screen.queryByText(stableId)).not.toBeInTheDocument()
    expect(screen.getAllByText("ANN rank 1")).toHaveLength(2)
  })

  it("enriches visible retrieval and context candidates through one shared preview batch", async () => {
    vi.mocked(resolveTextUnits).mockResolvedValue({
      resolved: [
        { id: "A", short_id: "a", preview: "王婆道：大官人若要成此事，只在我身上。", n_tokens: 42 },
        { id: "B", short_id: "b", preview: "西门庆次日清晨又来到王婆茶坊。", n_tokens: 31 },
        { id: "C", short_id: "c", preview: "王婆见他脚步儿勤。", n_tokens: 18 },
      ],
      missing_ids: [],
    })
    renderBasicWithEvidence()

    await waitFor(() => expect(screen.getAllByText("王婆道：大官人若要成此事，只在我身上。")).toHaveLength(2))
    expect(screen.getAllByText("ANN rank 2")).toHaveLength(2)
    expect(resolveTextUnits).toHaveBeenCalledTimes(1)
    expect(resolveTextUnits).toHaveBeenCalledWith(["C", "A", "B"], expect.any(AbortSignal))
  })

  it("uses a compact stable-ID fallback when a text unit has no short ID", () => {
    const stableId = "71d89c81-1234-5678-90ab-12345678e942"
    renderBasic([
      envelope(1, "root", { type: "query_started", method: "basic" }),
      envelope(2, "retrieval", { type: "candidates_retrieved", record_type: "text_unit", candidates: [{ id: stableId, record_type: "text_unit", selected: false }] }, "root"),
    ])
    expect(screen.getByText("Text Unit")).toBeInTheDocument()
    expect(screen.getByText("ID: 71d89c81…5678e942")).toHaveClass("break-all")
    expect(screen.getByText("Source preview unavailable")).toBeInTheDocument()
    expect(screen.queryByText(stableId)).not.toBeInTheDocument()
  })

  it("prefers effective model names and falls back for historical events", () => {
    const { rerender } = renderBasic([
      envelope(1, "root", { type: "query_started", method: "basic" }),
      envelope(2, "llm", { type: "llm_request_started", model_id: "default_completion_model", model_name: "deepseek-v4-flash", provider: "deepseek", prompt_tokens: 1 }, "root"),
    ])
    expect(screen.getAllByText("deepseek-v4-flash").length).toBeGreaterThan(0)
    expect(screen.queryByText("default_completion_model")).not.toBeInTheDocument()
    rerender(<Timeline runId="basic-run" envelopes={[envelope(1, "root", { type: "query_started", method: "basic" }), envelope(2, "llm", { type: "llm_request_started", model_id: "default_completion_model", prompt_tokens: 1 }, "root")]} streamStatus="closed" onFocusGraph={vi.fn()} onInspectCandidate={vi.fn()} />)
    expect(screen.getAllByText("default_completion_model").length).toBeGreaterThan(0)
  })

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

  it("preserves Prompt, Context, and response content that collides with UI vocabulary", async () => {
    const user = userEvent.setup()
    await act(() => setStudioLocale("zh-CN", false))
    const events = basicEvents().map((item) => {
      if (item.record.event.type === "context_completed") return { ...item, record: { ...item.record, event: { ...item.record.event, context: "Relationships" } } }
      if (item.record.event.type === "llm_request_started") return { ...item, record: { ...item.record, event: { ...item.record.event, prompt: "Standard" } } }
      if (item.record.event.type === "llm_request_completed") return { ...item, record: { ...item.record, event: { ...item.record.event, response: "Detailed" } } }
      return item
    })
    renderBasic(events)

    await user.click(screen.getByRole("button", { name: "查看基础检索 Context" }))
    expect(screen.getByTestId("exact-basic-context")).toHaveTextContent("Relationships")
    await user.click(screen.getByRole("button", { name: "查看基础检索 Prompt" }))
    expect(screen.getByTestId("exact-basic-prompt")).toHaveTextContent("Standard")
    await user.click(screen.getByRole("button", { name: "查看基础检索原始响应" }))
    expect(screen.getByTestId("raw-basic-response")).toHaveTextContent("Detailed")
    expect(screen.queryByText("关系")).not.toBeInTheDocument()
    expect(screen.queryByText("标准")).not.toBeInTheDocument()
    expect(screen.queryByText("详细")).not.toBeInTheDocument()
  })

  it("hides empty content actions and shows one metadata-only notice", () => {
    renderBasic([envelope(0, "run", { type: "run_started", content_mode: "metadata" }), ...basicEvents(false)])
    expect(screen.queryByRole("button", { name: "View Basic Context" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "View Basic Prompt" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "View Raw Basic Response" })).not.toBeInTheDocument()
    expect(screen.getAllByText("This Run recorded metadata only. Prompt, Context and model content are unavailable.")).toHaveLength(1)
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

  it("keeps raw events available only in Debug developer details", async () => {
    const user = userEvent.setup()
    renderBasic([envelope(0, "root", { type: "run_started", content_mode: "debug" }), ...basicEvents()])
    const retrieval = screen.getByRole("article", { name: "Text Retrieval" })
    await user.click(within(retrieval).getByRole("button", { name: /Developer details/ }))
    expect(within(retrieval).getByText("Embedding started")).toBeInTheDocument()
    await user.click(screen.getByText(/Developer events/))
    expect(screen.getByText("Query started")).toBeInTheDocument()
    expect(screen.getByText("Run completed")).toBeInTheDocument()
  })
})
