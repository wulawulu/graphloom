import type { ReactNode } from "react"
import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, ExplainabilityEventPayload, StartQueryResponse } from "@/api/types"
import { App } from "@/app"

const lifecycle = vi.hoisted(() => ({
  status: "running",
  envelopes: [] as ExplainabilityEnvelope[],
}))

function envelope(sequence: number, event: ExplainabilityEventPayload, runId = "run-new"): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: runId, timestamp: "2026-08-09T00:00:00Z", span_id: "span", event } }
}

vi.mock("@/components/graph/graph-explorer", () => ({
  GraphExplorer: ({ emphasisIntent, focusIntent, inspectIntent, navigationResetRevision, runId }: { emphasisIntent: { entityIds: string[]; relationshipIds: string[]; revision: number } | null; focusIntent: { entity_ids: string[]; relationship_ids: string[]; revision: number } | null; inspectIntent: { candidate: { stableId: string }; revision: number } | null; navigationResetRevision: number; runId: string | null }) => (
    <div>{focusIntent === null ? "Graph overview" : `Graph focus ${focusIntent.entity_ids.join(",")}/${focusIntent.relationship_ids.join(",")} revision ${focusIntent.revision}`}<span>Graph run {runId ?? "none"}</span><span>Navigation revision {navigationResetRevision}</span><span>{emphasisIntent === null ? "No citation emphasis" : `Citation emphasis ${emphasisIntent.entityIds.join(",")}/${emphasisIntent.relationshipIds.join(",")} revision ${emphasisIntent.revision}`}</span><span>{inspectIntent === null ? "No candidate inspection" : `Candidate inspection ${inspectIntent.candidate.stableId} revision ${inspectIntent.revision}`}</span></div>
  ),
}))

vi.mock("@/components/layout/studio-shell", () => ({
  StudioShell: ({ queryWorkspace, graph }: { queryWorkspace: ReactNode; graph: ReactNode }) => <div>{queryWorkspace}{graph}</div>,
}))

vi.mock("@/components/workspace/qa-workspace", () => ({
  QaWorkspace: ({ answer, composer, onFocusGraph, onInspectCandidate, onNewQuery, onSelectRun, question, runId, envelopes }: {
    answer: ReactNode
    composer: ReactNode
    onFocusGraph: (envelope: ExplainabilityEnvelope) => void
    onInspectCandidate: (candidate: { stableId: string; title: string; recordType: string; selected: boolean; selectionStatus: string; finalContext: string }) => void
    onNewQuery: () => void
    onSelectRun: (runId: string) => void
    question: string | null
    runId: string | null
    envelopes: ExplainabilityEnvelope[]
  }) => <div>{composer}{answer}<span>Workspace run {runId ?? "none"}</span><span>Question {question ?? "none"}</span><button type="button" onClick={onNewQuery}>New Query test</button><button type="button" onClick={() => onSelectRun("run-a")}>Select Run A</button><button type="button" onClick={() => onSelectRun("run-b")}>Select Run B</button><button type="button" onClick={() => onInspectCandidate({ stableId: "entity-candidate", title: "Candidate", recordType: "entity", selected: true, selectionStatus: "selected", finalContext: "included" })}>Inspect candidate test</button>{envelopes[0] === undefined ? null : <button type="button" onClick={() => onFocusGraph(envelopes[0]!)}>Show in graph</button>}</div>,
}))

vi.mock("@/components/query/query-composer", () => ({
  QueryComposer: ({ onAccepted }: { onAccepted: (response: StartQueryResponse, query: string) => void }) => (
    <button type="button" onClick={() => onAccepted({ run_id: "run-new", run_url: "", events_url: "", result_url: "" }, "Fresh question")}>Accept new Query</button>
  ),
}))

vi.mock("@/components/result/answer-panel", () => ({
  AnswerPanel: ({ onCitationEmphasis }: { onCitationEmphasis: (value: { entityIds: string[]; relationshipIds: string[] }) => void }) => <button type="button" onClick={() => onCitationEmphasis({ entityIds: ["entity-citation"], relationshipIds: [] })}>Citation test</button>,
}))

vi.mock("@/hooks/use-explainability-stream", () => ({
  useExplainabilityStream: () => ({ envelopes: lifecycle.envelopes, status: lifecycle.status === "running" ? "open" : "closed" }),
}))

vi.mock("@/hooks/use-run", () => ({
  useRun: (runId: string | null) => ({
    run: runId === null ? null : { run_id: runId, kind: "query", status: lifecycle.status, started_at: "2026-08-09T00:00:00Z", event_count: lifecycle.envelopes.length },
    result: { state: lifecycle.status === "completed" ? "ready" : "waiting", result: { run_id: runId, response: "answer", elapsed_ms: 1, usage: { llm_calls: 1, prompt_tokens: 1, output_tokens: 1, categories: {} } } },
    loading: false,
    refresh: vi.fn(),
  }),
}))

vi.mock("@/hooks/use-run-history", () => ({
  useRunHistory: () => ({ runs: [], loading: false, error: null, cursor: null, refresh: vi.fn(), loadMore: vi.fn() }),
}))

beforeEach(() => {
  lifecycle.status = "running"
  lifecycle.envelopes = []
})
afterEach(cleanup)

describe("App graph focus ownership", () => {
  it("clears the previous Query focus when selecting a historical Run", async () => {
    const user = userEvent.setup()
    lifecycle.envelopes = [envelope(1, { type: "entities_selected", entities: [{ id: "entity-a", selected: true }] }, "run-a")]
    render(<App />)

    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Show in graph" }))
    expect(await screen.findByText("Graph focus entity-a/ revision 1")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Select Run B" }))
    expect(screen.getByText("Graph run run-b")).toBeInTheDocument()
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
  })

  it("clears the previous focus when a new Query is accepted and shows its ephemeral question", async () => {
    const user = userEvent.setup()
    lifecycle.envelopes = [envelope(1, { type: "entities_selected", entities: [{ id: "entity-a", selected: true }] }, "run-a")]
    render(<App />)

    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Show in graph" }))
    await user.click(screen.getByRole("button", { name: "Accept new Query" }))
    expect(screen.getByText("Graph run run-new")).toBeInTheDocument()
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
    expect(screen.getByText("Question Fresh question")).toBeInTheDocument()
  })

  it("prepares a New Query without clearing the current graph", async () => {
    const user = userEvent.setup()
    lifecycle.envelopes = [envelope(1, { type: "entities_selected", entities: [{ id: "entity-a", selected: true }] }, "run-a")]
    render(<App />)
    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Show in graph" }))
    const revisionBefore = screen.getByText(/Navigation revision/).textContent

    await user.click(screen.getByRole("button", { name: "New Query test" }))
    expect(screen.getByText("Workspace run none")).toBeInTheDocument()
    expect(screen.getByText("Graph run run-a")).toBeInTheDocument()
    expect(screen.getByText("Graph focus entity-a/ revision 1")).toBeInTheDocument()
    expect(screen.getByText(/Navigation revision/).textContent).not.toBe(revisionBefore)
  })

  it("waits for successful terminal completion, then auto-focuses the combined evidence exactly once", async () => {
    const user = userEvent.setup()
    const view = render(<App />)
    await user.click(screen.getByRole("button", { name: "Accept new Query" }))

    lifecycle.envelopes = [
      envelope(1, { type: "entities_selected", entities: [{ id: "entity-a", selected: true }, { id: "entity-rejected", selected: false }] }),
      envelope(2, { type: "graph_expansion_started", seed_entity_ids: ["entity-seed", "entity-a"] }),
      envelope(3, { type: "relationships_selected", relationships: [{ id: "relationship-a", selected: true }, { id: "relationship-rejected", selected: false }] }),
    ]
    view.rerender(<App />)
    expect(screen.getByText("Graph overview")).toBeInTheDocument()

    lifecycle.status = "completed"
    lifecycle.envelopes = [...lifecycle.envelopes, envelope(4, { type: "run_completed" })]
    view.rerender(<App />)
    expect(await screen.findByText("Graph focus entity-a,entity-seed/relationship-a revision 1")).toBeInTheDocument()

    lifecycle.envelopes = [...lifecycle.envelopes, envelope(4, { type: "run_completed" })]
    view.rerender(<App />)
    expect(screen.getByText("Graph focus entity-a,entity-seed/relationship-a revision 1")).toBeInTheDocument()
    expect(screen.queryByText(/revision 2/)).not.toBeInTheDocument()
  })

  it.each([
    ["failed", [envelope(1, { type: "entities_selected", entities: [{ id: "entity-a", selected: true }] }), envelope(2, { type: "run_failed" })]],
    ["completed", [envelope(1, { type: "run_completed" })]],
  ])("does not auto-focus a %s Run without successful graph evidence", async (status, envelopes) => {
    const user = userEvent.setup()
    const view = render(<App />)
    await user.click(screen.getByRole("button", { name: "Accept new Query" }))
    lifecycle.status = status
    lifecycle.envelopes = envelopes
    view.rerender(<App />)
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
  })

  it("does not auto-focus a completed historical Run", async () => {
    const user = userEvent.setup()
    render(<App />)
    lifecycle.status = "completed"
    lifecycle.envelopes = [envelope(1, { type: "entities_selected", entities: [{ id: "entity-a", selected: true }] }), envelope(2, { type: "run_completed" })]
    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
  })

  it("keeps citation emphasis independent from graph focus and clears it on Run switch", async () => {
    const user = userEvent.setup()
    render(<App />)
    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Citation test" }))

    expect(screen.getByText("Graph overview")).toBeInTheDocument()
    expect(screen.getByText("Citation emphasis entity-citation/ revision 1")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Select Run B" }))
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
    expect(screen.getByText("No citation emphasis")).toBeInTheDocument()
  })

  it("keeps candidate inspection independent from graph focus and citation emphasis", async () => {
    const user = userEvent.setup()
    render(<App />)
    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Inspect candidate test" }))

    expect(screen.getByText("Candidate inspection entity-candidate revision 1")).toBeInTheDocument()
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
    expect(screen.getByText("No citation emphasis")).toBeInTheDocument()
  })

  it("starts candidate inspection revisions fresh after switching Runs", async () => {
    const user = userEvent.setup()
    render(<App />)
    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Inspect candidate test" }))
    expect(screen.getByText("Candidate inspection entity-candidate revision 1")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Select Run B" }))
    expect(screen.getByText("No candidate inspection")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Inspect candidate test" }))
    expect(screen.getByText("Candidate inspection entity-candidate revision 1")).toBeInTheDocument()
  })
})
