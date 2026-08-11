import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import {
  getCommunity,
  getCommunityReport,
  getEntity,
  getGraphOverview,
  getGraphSubgraph,
  getGraphSummary,
  getRelationship,
} from "@/api/client"
import type { GraphProjection } from "@/api/types"
import { GraphExplorer, type GraphFocusIntent } from "@/components/graph/graph-explorer"
import { highlightFromEvent } from "@/lib/explainability"

vi.mock("@/api/client", () => ({
  ApiError: class extends Error { status = 500 },
  getCommunity: vi.fn(),
  getCommunityReport: vi.fn(),
  getEntity: vi.fn(),
  getGraphOverview: vi.fn(),
  getGraphSubgraph: vi.fn(),
  getGraphSummary: vi.fn(),
  getRelationship: vi.fn(),
  listCommunities: vi.fn().mockResolvedValue({ items: [], next_cursor: null }),
  listEntities: vi.fn().mockResolvedValue({ items: [], next_cursor: null }),
  listRelationships: vi.fn().mockResolvedValue({ items: [], next_cursor: null }),
}))

vi.mock("@/components/graph/network-preview", () => ({
  NetworkPreview: ({ projection, mode, error, onBackOverview, onEntity, onRelationship, onReload }: { projection: GraphProjection; mode: string; error: string | null; onBackOverview: () => void; onEntity: (id: string) => void; onRelationship: (id: string) => void; onReload: () => void }) => <div><span>{mode}</span>{projection.entities.map((entity) => <span key={entity.id}>{entity.title}</span>)}{error === null ? null : <span>{error}</span>}{mode !== "overview" ? <button onClick={onBackOverview}>Back test</button> : null}<button onClick={() => onEntity("entity-1")}>Open entity test</button><button onClick={() => onRelationship("relationship-1")}>Open relationship test</button><button onClick={onReload}>Reload test</button></div>,
}))

const summary = { entity_count: 10, relationship_count: 20, community_count: 2, community_report_count: 2, community_levels: [0], entity_types: { PERSON: 1 }, untyped_entity_count: 0 }

function projection(id: string, seed = false): GraphProjection {
  return {
    entities: [{ id, title: id.toUpperCase(), entity_type: "PERSON", rank: 1 }],
    relationships: [],
    seed_entity_ids: seed ? [id] : [],
    seed_relationship_ids: [],
    missing_entity_ids: [],
    missing_relationship_ids: [],
    unresolved_relationship_ids: [],
    unresolved_relationship_count: 0,
    truncated: false,
  }
}

const defaultExplorerProps = { runId: "run-a", onClearFocus: vi.fn() }

beforeEach(() => {
  defaultExplorerProps.onClearFocus.mockReset()
  vi.mocked(getGraphSummary).mockReset().mockResolvedValue(summary)
  vi.mocked(getGraphOverview).mockReset().mockResolvedValue(projection("overview"))
  vi.mocked(getGraphSubgraph).mockReset()
  vi.mocked(getEntity).mockReset()
  vi.mocked(getRelationship).mockReset()
  vi.mocked(getCommunity).mockReset()
  vi.mocked(getCommunityReport).mockReset()
})
afterEach(cleanup)

describe("GraphExplorer focus flow", () => {
  it("posts entity and relationship seeds with bounded depth-one defaults", async () => {
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    const intent: GraphFocusIntent = { entity_ids: ["entity-1"], relationship_ids: ["relationship-1"], depth: 1, max_entities: 80, max_relationships: 160, revision: 1 }
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={intent} />)

    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    expect(vi.mocked(getGraphSubgraph).mock.calls[0]?.[0]).toEqual({ entity_ids: ["entity-1"], relationship_ids: ["relationship-1"], depth: 1, max_entities: 80, max_relationships: 160 })
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()
    expect(screen.getByText("query-focus")).toBeInTheDocument()
  })

  it("posts only selected candidate IDs and excludes rejected records", async () => {
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    const entities = highlightFromEvent({ type: "entities_selected", entities: [{ id: "entity-selected", selected: true }, { id: "entity-rejected", selected: false }] })
    const relationships = highlightFromEvent({ type: "relationships_selected", relationships: [{ id: "relationship-selected", selected: true }, { id: "relationship-rejected", selected: false }] })
    if (entities === null || relationships === null) throw new Error("selection fixtures must produce graph focus")
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={{ entity_ids: entities.entityIds, relationship_ids: relationships.relationshipIds, depth: 1, max_entities: 80, max_relationships: 160, revision: 1 }} />)

    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    expect(vi.mocked(getGraphSubgraph).mock.calls[0]?.[0]).toEqual({ entity_ids: ["entity-selected"], relationship_ids: ["relationship-selected"], depth: 1, max_entities: 80, max_relationships: 160 })
  })

  it("aborts stale focus A and lets focus B own the projection", async () => {
    let resolveA: ((value: GraphProjection) => void) | undefined
    const pendingA = new Promise<GraphProjection>((resolve) => { resolveA = resolve })
    vi.mocked(getGraphSubgraph).mockReturnValueOnce(pendingA).mockResolvedValueOnce(projection("b", true))
    const focusA: GraphFocusIntent = { entity_ids: ["a"], relationship_ids: [], revision: 1 }
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={focusA} />)
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    const signalA = vi.mocked(getGraphSubgraph).mock.calls[0]?.[1]

    rerender(<GraphExplorer {...defaultExplorerProps} focusIntent={{ entity_ids: ["b"], relationship_ids: [], revision: 2 }} />)
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(2))
    expect(signalA?.aborted).toBe(true)
    expect(await screen.findByText("B")).toBeInTheDocument()
    resolveA?.(projection("a", true))
    await Promise.resolve()
    expect(screen.queryByText("A")).not.toBeInTheDocument()
  })

  it("loads only the new Query focus when Run and focus intent change together", async () => {
    vi.mocked(getGraphSubgraph)
      .mockResolvedValueOnce(projection("focus-a", true))
      .mockResolvedValueOnce(projection("focus-b", true))
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={{ entity_ids: ["a"], relationship_ids: [], revision: 1 }} />)
    expect(await screen.findByText("FOCUS-A")).toBeInTheDocument()

    rerender(<GraphExplorer runId="run-b" focusIntent={{ entity_ids: ["b"], relationship_ids: [], revision: 2 }} onClearFocus={defaultExplorerProps.onClearFocus} />)

    expect(await screen.findByText("FOCUS-B")).toBeInTheDocument()
    expect(getGraphSubgraph).toHaveBeenCalledTimes(2)
    expect(getGraphOverview).not.toHaveBeenCalled()
    expect(screen.getByText("query-focus")).toBeInTheDocument()
  })

  it("invalidates a committed Query focus when its unchanged intent survives a Run change", async () => {
    const intent: GraphFocusIntent = { entity_ids: ["focus-a"], relationship_ids: [], revision: 1 }
    const onClearFocus = vi.fn()
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focus-a", true))
    vi.mocked(getGraphOverview).mockResolvedValue(projection("overview-b"))
    const { rerender } = render(<GraphExplorer runId="run-a" focusIntent={intent} onClearFocus={onClearFocus} />)
    expect(await screen.findByText("FOCUS-A")).toBeInTheDocument()

    rerender(<GraphExplorer runId="run-b" focusIntent={intent} onClearFocus={onClearFocus} />)

    expect(await screen.findByText("OVERVIEW-B")).toBeInTheDocument()
    expect(onClearFocus).toHaveBeenCalledOnce()
    expect(getGraphSubgraph).toHaveBeenCalledTimes(1)
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(screen.queryByText("FOCUS-A")).not.toBeInTheDocument()
    rerender(<GraphExplorer runId="run-b" focusIntent={null} onClearFocus={onClearFocus} />)
    await Promise.resolve()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
  })

  it("aborts a pending unchanged Query focus when the Run changes", async () => {
    let resolveFocus: ((value: GraphProjection) => void) | undefined
    const intent: GraphFocusIntent = { entity_ids: ["focus-a"], relationship_ids: [], revision: 1 }
    const onClearFocus = vi.fn()
    vi.mocked(getGraphSubgraph).mockReturnValue(new Promise((resolve) => { resolveFocus = resolve }))
    vi.mocked(getGraphOverview).mockResolvedValue(projection("overview-b"))
    const { rerender } = render(<GraphExplorer runId="run-a" focusIntent={intent} onClearFocus={onClearFocus} />)
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    const focusSignal = vi.mocked(getGraphSubgraph).mock.calls[0]?.[1]

    rerender(<GraphExplorer runId="run-b" focusIntent={intent} onClearFocus={onClearFocus} />)

    expect(await screen.findByText("OVERVIEW-B")).toBeInTheDocument()
    expect(focusSignal?.aborted).toBe(true)
    expect(onClearFocus).toHaveBeenCalledOnce()
    resolveFocus?.(projection("stale-query-focus", true))
    await Promise.resolve()
    expect(screen.queryByText("STALE-QUERY-FOCUS")).not.toBeInTheDocument()
    rerender(<GraphExplorer runId="run-b" focusIntent={null} onClearFocus={onClearFocus} />)
    await Promise.resolve()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("returns from focus mode to a freshly loaded overview", async () => {
    vi.mocked(getGraphOverview).mockResolvedValueOnce(projection("overview-new"))
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    const user = userEvent.setup()
    const onClearFocus = vi.fn()
    const { rerender } = render(<GraphExplorer runId="run-a" focusIntent={{ entity_ids: ["focused"], relationship_ids: [], revision: 1 }} onClearFocus={onClearFocus} />)
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Back test" }))
    expect(onClearFocus).toHaveBeenCalledOnce()
    rerender(<GraphExplorer runId="run-a" focusIntent={null} onClearFocus={onClearFocus} />)

    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(getGraphSummary).toHaveBeenCalledTimes(1)
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("returns to overview when the focus intent is cleared", async () => {
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    vi.mocked(getGraphOverview).mockResolvedValue(projection("overview-after-focus"))
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={{ entity_ids: ["focused"], relationship_ids: [], revision: 1 }} />)
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()

    rerender(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)

    expect(await screen.findByText("OVERVIEW-AFTER-FOCUS")).toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
  })

  it("aborts a pending focus when cleared and lets overview own the projection", async () => {
    let resolveFocus: ((value: GraphProjection) => void) | undefined
    vi.mocked(getGraphSubgraph).mockReturnValue(new Promise((resolve) => { resolveFocus = resolve }))
    vi.mocked(getGraphOverview).mockResolvedValue(projection("overview-new"))
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={{ entity_ids: ["focused"], relationship_ids: [], revision: 1 }} />)
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    const focusSignal = vi.mocked(getGraphSubgraph).mock.calls[0]?.[1]

    rerender(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)

    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(focusSignal?.aborted).toBe(true)
    resolveFocus?.(projection("stale-focus", true))
    await Promise.resolve()
    expect(screen.queryByText("STALE-FOCUS")).not.toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("returns a manual entity focus to overview when the Run changes", async () => {
    vi.mocked(getGraphOverview)
      .mockResolvedValueOnce(projection("overview-a"))
      .mockResolvedValueOnce(projection("overview-b"))
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1",
      short_id: "E1",
      title: "Alice",
      entity_type: "PERSON",
      rank: 1,
      description: "Alice description",
      community_ids: [],
      text_unit_ids: [],
    })
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("manual-focus", true))
    const user = userEvent.setup()
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW-A")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    await user.click(await screen.findByRole("button", { name: "Focus neighborhood" }))
    expect(await screen.findByText("MANUAL-FOCUS")).toBeInTheDocument()
    expect(screen.getByText("explorer-focus")).toBeInTheDocument()

    rerender(<GraphExplorer runId="run-b" focusIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)
    await waitFor(() => expect(getGraphOverview).toHaveBeenCalledTimes(2))
    expect(vi.mocked(getGraphOverview).mock.calls[1]?.[1]?.aborted).toBe(false)
    expect(await screen.findByText("OVERVIEW-B")).toBeInTheDocument()
    expect(screen.queryByText("MANUAL-FOCUS")).not.toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(2)
  })

  it("aborts a pending manual focus on Run change and ignores its late result", async () => {
    let resolveFocus: ((value: GraphProjection) => void) | undefined
    vi.mocked(getGraphOverview)
      .mockResolvedValueOnce(projection("overview-a"))
      .mockResolvedValueOnce(projection("overview-b"))
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1",
      short_id: "E1",
      title: "Alice",
      entity_type: "PERSON",
      rank: 1,
      description: null,
      community_ids: [],
      text_unit_ids: [],
    })
    vi.mocked(getGraphSubgraph).mockReturnValue(new Promise((resolve) => { resolveFocus = resolve }))
    const user = userEvent.setup()
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW-A")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    await user.click(await screen.findByRole("button", { name: "Focus neighborhood" }))
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    const focusSignal = vi.mocked(getGraphSubgraph).mock.calls[0]?.[1]

    rerender(<GraphExplorer runId="run-b" focusIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)

    expect(await screen.findByText("OVERVIEW-B")).toBeInTheDocument()
    expect(focusSignal?.aborted).toBe(true)
    resolveFocus?.(projection("stale-manual-focus", true))
    await Promise.resolve()
    expect(screen.queryByText("STALE-MANUAL-FOCUS")).not.toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("uses explorer focus mode for relationship detail focus", async () => {
    vi.mocked(getRelationship).mockResolvedValue({
      id: "relationship-1",
      short_id: "R1",
      source: "Alice",
      target: "Acme",
      weight: 1,
      rank: 2,
      description: "works at",
      text_unit_ids: [],
    })
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("relationship-focus", true))
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Open relationship test" }))
    await user.click(await screen.findByRole("button", { name: "Focus relationship" }))

    expect(await screen.findByText("RELATIONSHIP-FOCUS")).toBeInTheDocument()
    expect(screen.getByText("explorer-focus")).toBeInTheDocument()
    expect(vi.mocked(getGraphSubgraph).mock.calls[0]?.[0]).toEqual({
      entity_ids: [],
      relationship_ids: ["relationship-1"],
      depth: 1,
      max_entities: 80,
      max_relationships: 160,
    })
  })

  it("returns manual focus directly to overview without clearing Query intent", async () => {
    vi.mocked(getGraphOverview)
      .mockResolvedValueOnce(projection("overview-old"))
      .mockResolvedValueOnce(projection("overview-new"))
    vi.mocked(getRelationship).mockResolvedValue({
      id: "relationship-1",
      short_id: "R1",
      source: "Alice",
      target: "Acme",
      weight: 1,
      rank: 2,
      description: "works at",
      text_unit_ids: [],
    })
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("relationship-focus", true))
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW-OLD")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open relationship test" }))
    await user.click(await screen.findByRole("button", { name: "Focus relationship" }))
    expect(await screen.findByText("RELATIONSHIP-FOCUS")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Close detail panel" }))

    await user.click(screen.getByRole("button", { name: "Back test" }))

    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(defaultExplorerProps.onClearFocus).not.toHaveBeenCalled()
    expect(getGraphOverview).toHaveBeenCalledTimes(2)
  })

  it("does not reload overview when the Run changes while already in overview", async () => {
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()

    rerender(<GraphExplorer runId="run-b" focusIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)

    await Promise.resolve()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
  })

  it("keeps the current projection when focus loading fails", async () => {
    vi.mocked(getGraphSubgraph).mockRejectedValue(new Error("unavailable"))
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    rerender(<GraphExplorer {...defaultExplorerProps} focusIntent={{ entity_ids: ["missing"], relationship_ids: [], revision: 1 }} />)

    expect(await screen.findByText("Could not load focused graph.")).toBeInTheDocument()
    expect(screen.getByText("OVERVIEW")).toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("offers overview recovery when the initial focused graph fails", async () => {
    vi.mocked(getGraphSubgraph).mockRejectedValue(new Error("unavailable"))
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={{ entity_ids: ["missing"], relationship_ids: [], revision: 1 }} />)

    expect(await screen.findByText("Could not load focused graph.")).toBeInTheDocument()
    expect(getGraphOverview).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Load overview" }))

    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("keeps a successful overview usable when summary loading fails", async () => {
    vi.mocked(getGraphSummary).mockRejectedValue(new Error("summary unavailable"))
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)

    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    expect(screen.getByText("Graph summary is unavailable. The bounded visualization is still available.")).toBeInTheDocument()
  })

  it("reloads both the overview and summary", async () => {
    vi.mocked(getGraphSummary)
      .mockResolvedValueOnce({ ...summary, entity_count: 362 })
      .mockResolvedValueOnce({ ...summary, entity_count: 410 })
    vi.mocked(getGraphOverview)
      .mockResolvedValueOnce(projection("overview-old"))
      .mockResolvedValueOnce(projection("overview-new"))
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("362")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Reload test" }))

    expect(await screen.findByText("410")).toBeInTheDocument()
    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(getGraphSummary).toHaveBeenCalledTimes(2)
    expect(getGraphOverview).toHaveBeenCalledTimes(2)
  })

  it("recovers an unavailable summary without hiding the graph", async () => {
    vi.mocked(getGraphSummary)
      .mockRejectedValueOnce(new Error("summary unavailable"))
      .mockResolvedValueOnce({ ...summary, entity_count: 410 })
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    expect(screen.getByText("Graph summary is unavailable. The bounded visualization is still available.")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Reload test" }))

    expect(await screen.findByText("410")).toBeInTheDocument()
    expect(screen.queryByText("Graph summary is unavailable. The bounded visualization is still available.")).not.toBeInTheDocument()
    expect(screen.getByText("OVERVIEW")).toBeInTheDocument()
  })

  it("aborts a stale summary reload and keeps the latest summary", async () => {
    let resolveStaleSummary: ((value: typeof summary) => void) | undefined
    const staleSummary = new Promise<typeof summary>((resolve) => { resolveStaleSummary = resolve })
    vi.mocked(getGraphSummary)
      .mockResolvedValueOnce(summary)
      .mockReturnValueOnce(staleSummary)
      .mockResolvedValueOnce({ ...summary, entity_count: 420 })
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("10")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Reload test" }))
    await waitFor(() => expect(getGraphSummary).toHaveBeenCalledTimes(2))
    const staleSignal = vi.mocked(getGraphSummary).mock.calls[1]?.[0]
    await user.click(screen.getByRole("button", { name: "Reload test" }))

    expect(await screen.findByText("420")).toBeInTheDocument()
    expect(staleSignal?.aborted).toBe(true)
    resolveStaleSummary?.({ ...summary, entity_count: 410 })
    await Promise.resolve()
    expect(screen.queryByText("410")).not.toBeInTheDocument()
    expect(screen.getByText("420")).toBeInTheDocument()
  })
})
