import { act, cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { QaWorkspace } from "@/components/workspace/qa-workspace"

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

function envelope(sequence: number, event: ExplainabilityEventPayload, spanId = "span", parentSpanId?: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run-1", timestamp: "2026-08-09T00:00:00Z", span_id: spanId, parent_span_id: parentSpanId, event } }
}

const events = [
  envelope(1, { type: "run_started", content_mode: "debug" }),
  envelope(2, { type: "query_started", method: "local" }),
  envelope(3, { type: "entities_selected", entities: [{ id: "entity-1", title: "Alice", record_type: "entity", selected: true }] }),
]

function props() {
  return {
    runId: "run-1",
    runStatus: "completed",
    question: "How is Alice connected?",
    answer: <div>Authoritative answer</div>,
    answerHasStarted: false,
    isActiveSubmission: false,
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
  it("opens Analysis for a new Run and auto-collapses on the first answer token", () => {
    const values = props()
    const view = render(<QaWorkspace {...values} runStatus="running" isActiveSubmission answerHasStarted={false} />)
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()
    view.rerender(<QaWorkspace {...values} runStatus="running" isActiveSubmission answerHasStarted />)
    expect(screen.queryByRole("heading", { name: "Entity Mapping" })).not.toBeInTheDocument()
    expect(screen.getByText("Generating answer · 1 analysis step")).toBeInTheDocument()
  })

  it("respects a manual Analysis choice when the first answer token arrives", async () => {
    const user = userEvent.setup()
    const values = props()
    const view = render(<QaWorkspace {...values} runStatus="running" isActiveSubmission answerHasStarted={false} />)
    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()
    view.rerender(<QaWorkspace {...values} runStatus="running" isActiveSubmission answerHasStarted />)
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()
  })

  it("keeps Analysis open when a Run fails before any answer token", () => {
    const values = props()
    const view = render(<QaWorkspace {...values} runStatus="running" isActiveSubmission answerHasStarted={false} />)
    view.rerender(<QaWorkspace {...values} runStatus="failed" isActiveSubmission answerHasStarted={false} />)
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()
  })

  it("keeps Analysis open when a running history Run restores an existing partial snapshot", () => {
    const values = { ...props(), runId: "history-run" }
    const view = render(<QaWorkspace {...values} runStatus={undefined} isActiveSubmission={false} answerHasStarted={false} />)
    view.rerender(<QaWorkspace {...values} runStatus="running" isActiveSubmission={false} answerHasStarted={false} />)
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()

    view.rerender(<QaWorkspace {...values} runStatus="running" isActiveSubmission={false} answerHasStarted />)
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()

    view.rerender(<QaWorkspace {...values} runStatus="running" isActiveSubmission={false} answerHasStarted />)
    expect(screen.getByRole("heading", { name: "Entity Mapping" })).toBeInTheDocument()
  })

  it("shows one current Question and Answer with Analysis collapsed by default", () => {
    render(<QaWorkspace {...props()} />)

    const workspace = screen.getByRole("region", { name: "Graph QA workspace" })
    expect(within(workspace).getByText("How is Alice connected?")).toBeInTheDocument()
    expect(within(workspace).getByText("Authoritative answer")).toBeInTheDocument()
    expect(screen.getByText("Analysis · 1 step")).toBeInTheDocument()
    expect(screen.queryByText("Entities selected")).not.toBeInTheDocument()
    expect(screen.getByText("Bottom composer")).toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Query run history" })).not.toBeInTheDocument()
    const analysis = screen.getByText("Analysis · 1 step")
    const answer = screen.getByText("Authoritative answer")
    expect(analysis.compareDocumentPosition(answer) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0)
  })

  it("keeps terminal Run status authoritative while persisted events replay", () => {
    const values = props()
    const { rerender } = render(<QaWorkspace {...values} streamStatus="open" />)
    expect(screen.getByText("Analysis · 1 step")).toBeInTheDocument()

    rerender(<QaWorkspace {...values} runStatus="failed" streamStatus="connecting" />)
    expect(screen.getByText("Analysis interrupted · 1 step")).toBeInTheDocument()
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
    await user.click(screen.getByRole("button", { name: /Developer details/ }))
    await user.click(within(screen.getByRole("article", { name: "Entity Mapping" })).getByRole("button", { name: "Details" }))
    expect(screen.getByText("Selected 1 / Candidates 1")).toBeInTheDocument()
    await user.click(within(screen.getByRole("article", { name: "Entity Mapping" })).getAllByRole("button", { name: "Focus in graph" })[0]!)
    expect(values.onFocusGraph).toHaveBeenCalledWith(events[2])
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

  it("pauses follow when the user expands Analysis and returns smoothly on request", async () => {
    const user = userEvent.setup()
    const scrollTo = vi.spyOn(HTMLElement.prototype, "scrollTo")
    render(<QaWorkspace {...props()} />)

    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    const backToLatest = screen.getByRole("button", { name: "Back to latest" })
    expect(backToLatest).toBeInTheDocument()

    await user.click(backToLatest)
    expect(scrollTo).toHaveBeenLastCalledWith(expect.objectContaining({ behavior: "smooth" }))
    expect(screen.queryByRole("button", { name: "Back to latest" })).not.toBeInTheDocument()
  })

  it("resets follow for a new Query and when switching completed history Runs", async () => {
    const user = userEvent.setup()
    const values = props()
    const view = render(<QaWorkspace {...values} />)
    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByRole("button", { name: "Back to latest" })).toBeInTheDocument()

    view.rerender(<QaWorkspace {...values} runId="run-2" question="Second question" />)
    expect(screen.queryByRole("button", { name: "Back to latest" })).not.toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Toggle analysis process" }))
    expect(screen.getByRole("button", { name: "Back to latest" })).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "New Query" }))
    expect(values.onNewQuery).toHaveBeenCalledOnce()
    expect(screen.queryByRole("button", { name: "Back to latest" })).not.toBeInTheDocument()
  })

  it("keeps latest visible through Analysis collapse and canonical Answer reconciliation", () => {
    const animationFrames = new Map<number, FrameRequestCallback>()
    const observers: ResizeObserverCallback[] = []
    let nextFrame = 1
    vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
      const id = nextFrame
      nextFrame += 1
      animationFrames.set(id, callback)
      return id
    }))
    vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => animationFrames.delete(id)))
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: ResizeObserverCallback) { observers.push(callback) }
      disconnect(): void {}
      observe(): void {}
      unobserve(): void {}
    })
    const scrollTo = vi.spyOn(HTMLElement.prototype, "scrollTo")
    const values = props()
    const view = render(
      <QaWorkspace {...values} runStatus="running" isActiveSubmission answerHasStarted={false} answer={<div>Waiting</div>} />,
    )

    view.rerender(
      <QaWorkspace {...values} runStatus="running" isActiveSubmission answerHasStarted answer={<div>First token</div>} />,
    )
    act(() => observers.at(-1)?.([], {} as ResizeObserver))
    act(() => {
      const callbacks = [...animationFrames.values()]
      animationFrames.clear()
      callbacks.forEach((callback) => callback(0))
    })
    expect(screen.queryByRole("heading", { name: "Entity Mapping" })).not.toBeInTheDocument()
    expect(scrollTo).toHaveBeenLastCalledWith(expect.objectContaining({ behavior: "auto" }))

    scrollTo.mockClear()
    view.rerender(
      <QaWorkspace {...values} answerHasStarted answer={<div>Canonical answer and usage</div>} />,
    )
    act(() => observers.at(-1)?.([], {} as ResizeObserver))
    act(() => {
      const callbacks = [...animationFrames.values()]
      animationFrames.clear()
      callbacks.forEach((callback) => callback(0))
    })
    expect(screen.getByText("Canonical answer and usage")).toBeInTheDocument()
    expect(scrollTo).toHaveBeenCalledWith(expect.objectContaining({ behavior: "auto" }))
  })
})
