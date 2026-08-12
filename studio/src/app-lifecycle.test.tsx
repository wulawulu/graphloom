import type { ReactNode } from "react"
import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope, StartQueryResponse } from "@/api/types"
import { App } from "@/app"

vi.mock("@/components/explainability/timeline", () => ({
  Timeline: ({ onFocusGraph }: { onFocusGraph: (envelope: ExplainabilityEnvelope) => void }) => (
    <button type="button" onClick={() => onFocusGraph({ record: { event: {} } } as ExplainabilityEnvelope)}>Focus graph</button>
  ),
}))

vi.mock("@/components/graph/graph-explorer", () => ({
  GraphExplorer: ({ focusIntent, runId }: { focusIntent: { entity_ids: string[] } | null; runId: string | null }) => (
    <div>{focusIntent === null ? "Graph overview" : `Graph focus ${focusIntent.entity_ids.join(",")}`}<span>Graph run {runId ?? "none"}</span></div>
  ),
}))

vi.mock("@/components/layout/studio-shell", () => ({
  StudioShell: ({ queryWorkspace, graph }: { queryWorkspace: ReactNode; graph: ReactNode }) => (
    <div>{queryWorkspace}{graph}</div>
  ),
}))

vi.mock("@/components/workspace/query-workspace", () => ({
  QueryWorkspace: ({ composer, answer, trace, runs }: { composer: ReactNode; answer: ReactNode; trace: ReactNode; runs: ReactNode }) => <div>{composer}{answer}{trace}{runs}</div>,
}))

vi.mock("@/components/query/query-composer", () => ({
  QueryComposer: ({ onAccepted }: { onAccepted: (response: StartQueryResponse) => void }) => (
    <button type="button" onClick={() => onAccepted({ run_id: "run-new", run_url: "", events_url: "", result_url: "" })}>Accept new Query</button>
  ),
}))

vi.mock("@/components/result/answer-panel", () => ({
  AnswerPanel: () => null,
}))

vi.mock("@/components/runs/run-list", () => ({
  RunList: ({ selectedRunId, onSelect }: { selectedRunId: string | null; onSelect: (runId: string) => void }) => (
    <div>
      <span>Selected {selectedRunId ?? "none"}</span>
      <button type="button" onClick={() => onSelect("run-a")}>Select Run A</button>
      <button type="button" onClick={() => onSelect("run-b")}>Select Run B</button>
    </div>
  ),
}))

vi.mock("@/hooks/use-explainability-stream", () => ({
  useExplainabilityStream: () => ({ envelopes: [], status: "idle" }),
}))

vi.mock("@/hooks/use-run", () => ({
  useRun: () => ({ run: null, result: null, loading: false, refresh: vi.fn() }),
}))

vi.mock("@/hooks/use-run-history", () => ({
  useRunHistory: () => ({ runs: [], loading: false, error: null, cursor: null, refresh: vi.fn(), loadMore: vi.fn() }),
}))

vi.mock("@/lib/explainability", () => ({
  highlightFromEvent: () => ({ entityIds: ["entity-a"], relationshipIds: [] }),
}))

afterEach(cleanup)

describe("App graph focus ownership", () => {
  it("clears the previous Query focus when selecting a historical Run", async () => {
    const user = userEvent.setup()
    render(<App />)

    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Focus graph" }))
    expect(await screen.findByText("Graph focus entity-a")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Select Run B" }))
    expect(screen.getByText("Selected run-b")).toBeInTheDocument()
    expect(screen.getByText("Graph run run-b")).toBeInTheDocument()
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
    expect(screen.queryByText("Graph focus entity-a")).not.toBeInTheDocument()
  })

  it("clears the previous Query focus when a new Query is accepted", async () => {
    const user = userEvent.setup()
    render(<App />)

    await user.click(screen.getByRole("button", { name: "Select Run A" }))
    await user.click(screen.getByRole("button", { name: "Focus graph" }))
    expect(await screen.findByText("Graph focus entity-a")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Accept new Query" }))
    expect(screen.getByText("Selected run-new")).toBeInTheDocument()
    expect(screen.getByText("Graph run run-new")).toBeInTheDocument()
    expect(screen.getByText("Graph overview")).toBeInTheDocument()
    expect(screen.queryByText("Graph focus entity-a")).not.toBeInTheDocument()
  })
})
