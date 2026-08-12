import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { QaWorkspace } from "@/components/workspace/qa-workspace"

afterEach(cleanup)

function envelope(sequence: number, event: ExplainabilityEventPayload): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run-1", timestamp: "2026-08-09T00:00:00Z", span_id: "span", event } }
}

const events = [
  envelope(1, { type: "query_started" }),
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
    expect(screen.getByText("Analysis process · 2 steps · completed")).toBeInTheDocument()
    expect(screen.queryByText("Entities selected")).not.toBeInTheDocument()
    expect(screen.getByText("Bottom composer")).toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Query run history" })).not.toBeInTheDocument()
  })

  it("keeps terminal Run status authoritative while persisted events replay", () => {
    const values = props()
    const { rerender } = render(<QaWorkspace {...values} streamStatus="open" />)
    expect(screen.getByText("Analysis process · 2 steps · completed")).toBeInTheDocument()

    rerender(<QaWorkspace {...values} runStatus="failed" streamStatus="connecting" />)
    expect(screen.getByText("Analysis process · 2 steps · failed")).toBeInTheDocument()
  })

  it("expands ordered events with typed Details and keeps manual graph focus", async () => {
    const user = userEvent.setup()
    const values = props()
    render(<QaWorkspace {...values} />)

    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    const labels = screen.getAllByText(/Query started|Entities selected/)
    expect(labels.map((element) => element.textContent)).toEqual(["Query started", "Entities selected"])
    await user.click(screen.getAllByRole("button", { name: "Details" })[1]!)
    expect(screen.getByText("Selected 1 / Candidates 1")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Focus in graph" }))
    expect(values.onFocusGraph).toHaveBeenCalledWith(events[1])
  })

  it("retains streamed envelopes while Analysis is collapsed", async () => {
    const user = userEvent.setup()
    const values = props()
    const { rerender } = render(<QaWorkspace {...values} envelopes={[events[0]!]} />)

    rerender(<QaWorkspace {...values} envelopes={events} />)
    expect(screen.queryByText("Entities selected")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByText("Entities selected")).toBeInTheDocument()
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
