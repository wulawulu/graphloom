import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { QaWorkspace } from "@/components/workspace/qa-workspace"

afterEach(cleanup)

function envelope(sequence: number, event: ExplainabilityEventPayload, spanId = "span", parentSpanId?: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run-1", timestamp: "2026-08-09T00:00:00Z", span_id: spanId, parent_span_id: parentSpanId, event } }
}

const events = [
  envelope(1, { type: "query_started", method: "local" }),
  envelope(2, { type: "entities_selected", entities: [{ id: "entity-1", title: "Alice", record_type: "entity", selected: true }] }),
]

function props() {
  return {
    runId: "run-1",
    runStatus: "completed",
    question: "How is Alice connected?",
    answer: <div>Authoritative answer</div>,
    composer: <div>Bottom composer</div>,
    envelopes: events,
    streamStatus: "closed" as const,
    runs: [{ run_id: "run-hidden", kind: "query", status: "completed", query_method: "local", started_at: "2026-08-09T00:00:00Z", event_count: 4 }],
    historyLoading: false,
    historyError: null,
    historyHasMore: true,
    onFocusGraph: vi.fn(),
    onInspectCandidate: vi.fn(),
    onNewQuery: vi.fn(),
    onSelectRun: vi.fn(),
    onRefreshHistory: vi.fn(),
    onLoadMoreHistory: vi.fn(),
  }
}

describe("QaWorkspace", () => {
  it("shows one current Question and Answer with Analysis collapsed by default", () => {
    render(<QaWorkspace {...props()} />)

    const workspace = screen.getByRole("region", { name: "Graph QA workspace" })
    expect(within(workspace).getByText("How is Alice connected?")).toBeInTheDocument()
    expect(within(workspace).getByText("Authoritative answer")).toBeInTheDocument()
    expect(screen.getByText("Analysis process · 1 decision · completed")).toBeInTheDocument()
    expect(screen.queryByText("Entities selected")).not.toBeInTheDocument()
    expect(screen.getByText("Bottom composer")).toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Query run history" })).not.toBeInTheDocument()
    const analysis = screen.getByText("Analysis process · 1 decision · completed")
    const answer = screen.getByText("Authoritative answer")
    expect(analysis.compareDocumentPosition(answer) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0)
  })

  it("keeps terminal Run status authoritative while persisted events replay", () => {
    const values = props()
    const { rerender } = render(<QaWorkspace {...values} streamStatus="open" />)
    expect(screen.getByText("Analysis process · 1 decision · completed")).toBeInTheDocument()

    rerender(<QaWorkspace {...values} runStatus="failed" streamStatus="connecting" />)
    expect(screen.getByText("Analysis process · 1 decision · failed")).toBeInTheDocument()
  })

  it("expands semantic decisions, inspects candidates, and keeps graph focus explicit", async () => {
    const user = userEvent.setup()
    const values = props()
    render(<QaWorkspace {...values} />)

    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()
    expect(screen.getByText("Query started")).not.toBeVisible()
    await user.click(screen.getByRole("button", { name: "Inspect entity Alice" }))
    expect(values.onInspectCandidate).toHaveBeenCalledWith(expect.objectContaining({ stableId: "entity-1" }))
    expect(values.onFocusGraph).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: /Technical details/ }))
    await user.click(within(screen.getByRole("article", { name: "Entity Mapping" })).getByRole("button", { name: "Details" }))
    expect(screen.getByText("Selected 1 / Candidates 1")).toBeInTheDocument()
    await user.click(within(screen.getByRole("article", { name: "Entity Mapping" })).getAllByRole("button", { name: "Focus in graph" })[0]!)
    expect(values.onFocusGraph).toHaveBeenCalledWith(events[1])
  })

  it("retains streamed envelopes while Analysis is collapsed", async () => {
    const user = userEvent.setup()
    const values = props()
    const { rerender } = render(<QaWorkspace {...values} envelopes={[events[0]!]} />)

    rerender(<QaWorkspace {...values} envelopes={events} />)
    expect(screen.queryByText("Entity Mapping")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()
  })

  it("switches Local, Global, Dynamic Global, and Basic presentations without retaining expanded analysis", async () => {
    const user = userEvent.setup()
    const values = props()
    const view = render(<QaWorkspace {...values} />)
    expect(screen.getByText("Local")).toBeInTheDocument()

    const globalEnvelopes = [
      envelope(1, { type: "query_started", method: "global" }, "root"),
      envelope(2, { type: "global_map_started", batch_count: 1 }, "map", "root"),
      envelope(3, { type: "global_map_batch_built", batch_index: 0, report_count: 1, report_ids: ["report-1"], tokens_used: 10, token_budget: 20 }, "batch-0", "map"),
    ]
    view.rerender(<QaWorkspace {...values} runId="global-run" envelopes={globalEnvelopes} />)
    expect(screen.getByText("Global")).toBeInTheDocument()
    expect(screen.queryByRole("heading", { name: "Community Context" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByRole("heading", { name: "Community Context" })).toBeInTheDocument()

    const dynamicEnvelopes = [
      ...globalEnvelopes,
      envelope(4, { type: "dynamic_community_selection_started", initial_community_count: 1, threshold: 3, max_level: 2, keep_parent: false, use_summary: false, num_repeats: 1 }, "selection", "root"),
    ]
    view.rerender(<QaWorkspace {...values} runId="dynamic-run" envelopes={dynamicEnvelopes} />)
    expect(screen.getByText("Dynamic Global")).toBeInTheDocument()

    const basicEnvelopes = [
      envelope(1, { type: "query_started", method: "basic" }, "basic-root"),
      envelope(2, { type: "basic_retrieval_skipped", reason: "empty_query" }, "basic-retrieval", "basic-root"),
    ]
    view.rerender(<QaWorkspace {...values} runId="basic-run" envelopes={basicEnvelopes} />)
    expect(screen.getByText("Basic")).toBeInTheDocument()
    expect(screen.queryByRole("heading", { name: "Community Context" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByRole("heading", { name: "Text Retrieval" })).toBeInTheDocument()

    view.rerender(<QaWorkspace {...values} runId="local-run" envelopes={events} />)
    expect(screen.getByText("Local")).toBeInTheDocument()
    expect(screen.queryByRole("heading", { name: "Community Context" })).not.toBeInTheDocument()
  })

  it("opens History on demand, preserves metadata privacy, and selects through the existing callback", async () => {
    const user = userEvent.setup()
    const values = props()
    render(<QaWorkspace {...values} />)

    expect(screen.queryByText("Recent Runs")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "History" }))
    expect(screen.getByText("Recent Runs")).toBeInTheDocument()
    expect(screen.getByText("Query hidden (metadata mode)")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Load more" })).toBeInTheDocument()
    await user.click(screen.getByText("Query hidden (metadata mode)"))
    expect(values.onSelectRun).toHaveBeenCalledWith("run-hidden")
    expect(screen.queryByText("Recent Runs")).not.toBeInTheDocument()
  })
})
