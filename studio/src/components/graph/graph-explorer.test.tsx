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
  NetworkPreview: ({ projection, mode, error, onBackOverview, onReload }: { projection: GraphProjection; mode: string; error: string | null; onBackOverview: () => void; onReload: () => void }) => <div><span>{mode}</span>{projection.entities.map((entity) => <span key={entity.id}>{entity.title}</span>)}{error === null ? null : <span>{error}</span>}{mode === "focus" ? <button onClick={onBackOverview}>Back test</button> : null}<button onClick={onReload}>Reload test</button></div>,
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

beforeEach(() => {
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
    render(<GraphExplorer focusIntent={intent} />)

    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    expect(vi.mocked(getGraphSubgraph).mock.calls[0]?.[0]).toEqual({ entity_ids: ["entity-1"], relationship_ids: ["relationship-1"], depth: 1, max_entities: 80, max_relationships: 160 })
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()
    expect(screen.getByText("focus")).toBeInTheDocument()
  })

  it("posts only selected candidate IDs and excludes rejected records", async () => {
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    const entities = highlightFromEvent({ type: "entities_selected", entities: [{ id: "entity-selected", selected: true }, { id: "entity-rejected", selected: false }] })
    const relationships = highlightFromEvent({ type: "relationships_selected", relationships: [{ id: "relationship-selected", selected: true }, { id: "relationship-rejected", selected: false }] })
    if (entities === null || relationships === null) throw new Error("selection fixtures must produce graph focus")
    render(<GraphExplorer focusIntent={{ entity_ids: entities.entityIds, relationship_ids: relationships.relationshipIds, depth: 1, max_entities: 80, max_relationships: 160, revision: 1 }} />)

    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    expect(vi.mocked(getGraphSubgraph).mock.calls[0]?.[0]).toEqual({ entity_ids: ["entity-selected"], relationship_ids: ["relationship-selected"], depth: 1, max_entities: 80, max_relationships: 160 })
  })

  it("aborts stale focus A and lets focus B own the projection", async () => {
    let resolveA: ((value: GraphProjection) => void) | undefined
    const pendingA = new Promise<GraphProjection>((resolve) => { resolveA = resolve })
    vi.mocked(getGraphSubgraph).mockReturnValueOnce(pendingA).mockResolvedValueOnce(projection("b", true))
    const focusA: GraphFocusIntent = { entity_ids: ["a"], relationship_ids: [], revision: 1 }
    const { rerender } = render(<GraphExplorer focusIntent={focusA} />)
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    const signalA = vi.mocked(getGraphSubgraph).mock.calls[0]?.[1]

    rerender(<GraphExplorer focusIntent={{ entity_ids: ["b"], relationship_ids: [], revision: 2 }} />)
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(2))
    expect(signalA?.aborted).toBe(true)
    expect(await screen.findByText("B")).toBeInTheDocument()
    resolveA?.(projection("a", true))
    await Promise.resolve()
    expect(screen.queryByText("A")).not.toBeInTheDocument()
  })

  it("returns from focus mode to a freshly loaded overview", async () => {
    vi.mocked(getGraphOverview).mockResolvedValueOnce(projection("overview-new"))
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    const user = userEvent.setup()
    render(<GraphExplorer focusIntent={{ entity_ids: ["focused"], relationship_ids: [], revision: 1 }} />)
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Back test" }))

    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(getGraphSummary).toHaveBeenCalledTimes(1)
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("returns to overview when the focus intent is cleared", async () => {
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    vi.mocked(getGraphOverview).mockResolvedValue(projection("overview-after-focus"))
    const { rerender } = render(<GraphExplorer focusIntent={{ entity_ids: ["focused"], relationship_ids: [], revision: 1 }} />)
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()

    rerender(<GraphExplorer focusIntent={null} />)

    expect(await screen.findByText("OVERVIEW-AFTER-FOCUS")).toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
  })

  it("aborts a pending focus when cleared and lets overview own the projection", async () => {
    let resolveFocus: ((value: GraphProjection) => void) | undefined
    vi.mocked(getGraphSubgraph).mockReturnValue(new Promise((resolve) => { resolveFocus = resolve }))
    vi.mocked(getGraphOverview).mockResolvedValue(projection("overview-new"))
    const { rerender } = render(<GraphExplorer focusIntent={{ entity_ids: ["focused"], relationship_ids: [], revision: 1 }} />)
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(1))
    const focusSignal = vi.mocked(getGraphSubgraph).mock.calls[0]?.[1]

    rerender(<GraphExplorer focusIntent={null} />)

    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(focusSignal?.aborted).toBe(true)
    resolveFocus?.(projection("stale-focus", true))
    await Promise.resolve()
    expect(screen.queryByText("STALE-FOCUS")).not.toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("keeps the current projection when focus loading fails", async () => {
    vi.mocked(getGraphSubgraph).mockRejectedValue(new Error("unavailable"))
    const { rerender } = render(<GraphExplorer focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    rerender(<GraphExplorer focusIntent={{ entity_ids: ["missing"], relationship_ids: [], revision: 1 }} />)

    expect(await screen.findByText("Could not load focused graph.")).toBeInTheDocument()
    expect(screen.getByText("OVERVIEW")).toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("offers overview recovery when the initial focused graph fails", async () => {
    vi.mocked(getGraphSubgraph).mockRejectedValue(new Error("unavailable"))
    const user = userEvent.setup()
    render(<GraphExplorer focusIntent={{ entity_ids: ["missing"], relationship_ids: [], revision: 1 }} />)

    expect(await screen.findByText("Could not load focused graph.")).toBeInTheDocument()
    expect(getGraphOverview).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Load overview" }))

    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("keeps a successful overview usable when summary loading fails", async () => {
    vi.mocked(getGraphSummary).mockRejectedValue(new Error("summary unavailable"))
    render(<GraphExplorer focusIntent={null} />)

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
    render(<GraphExplorer focusIntent={null} />)
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
    render(<GraphExplorer focusIntent={null} />)
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
    render(<GraphExplorer focusIntent={null} />)
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
