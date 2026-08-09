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
  NetworkPreview: ({ projection, mode, error, onBackOverview }: { projection: GraphProjection; mode: string; error: string | null; onBackOverview: () => void }) => <div><span>{mode}</span>{projection.entities.map((entity) => <span key={entity.id}>{entity.title}</span>)}{error === null ? null : <span>{error}</span>}{mode === "focus" ? <button onClick={onBackOverview}>Back test</button> : null}</div>,
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
    vi.mocked(getGraphOverview).mockResolvedValueOnce(projection("overview")).mockResolvedValueOnce(projection("overview-new"))
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    const user = userEvent.setup()
    render(<GraphExplorer focusIntent={{ entity_ids: ["focused"], relationship_ids: [], revision: 1 }} />)
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Back test" }))

    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(getGraphOverview).toHaveBeenCalledTimes(2)
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
})
