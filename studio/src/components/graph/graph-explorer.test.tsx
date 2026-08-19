import { useEffect } from "react"
import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
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
import type { GraphEntityDetail, GraphProjection, GraphSummary } from "@/api/types"
import { GraphExplorer, type GraphFocusIntent, type GraphInspectIntent } from "@/components/graph/graph-explorer"
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

const networkLifecycle = vi.hoisted(() => ({ mounts: 0, unmounts: 0 }))

vi.mock("@/components/graph/network-preview", () => ({
  NetworkPreview: ({ projection, summary, summaryError, mode, error, onBack, backLabel, onEntity, onRelationship, onReload }: { projection: GraphProjection; summary: GraphSummary | null; summaryError: boolean; mode: string; error: string | null; onBack: () => void; backLabel: string; onEntity: (id: string) => void; onRelationship: (id: string) => void; onReload: () => void }) => {
    useEffect(() => {
      networkLifecycle.mounts += 1
      return () => { networkLifecycle.unmounts += 1 }
    }, [])
    return <div><span>{mode}</span>{projection.entities.map((entity) => <span key={entity.id}>{entity.title}</span>)}{summary === null ? null : <span>{summary.entity_count}</span>}{summaryError ? <span>Graph summary is unavailable. The bounded visualization is still available.</span> : null}{error === null ? null : <span>{error}</span>}{mode !== "overview" ? <><span>{backLabel}</span><button onClick={onBack}>Back test</button></> : null}<button onClick={() => onEntity("entity-1")}>Open entity test</button><button onClick={() => onRelationship("relationship-1")}>Open relationship test</button><button onClick={onReload}>Reload test</button></div>
  },
}))

const summary = { entity_count: 10, relationship_count: 20, community_count: 2, community_report_count: 2, community_levels: [0], entity_types: { PERSON: 1 }, untyped_entity_count: 0 }

function projection(id: string, seed = false): GraphProjection {
  return {
    entities: [{ id, title: id.toUpperCase(), entity_type: "PERSON", degree: 1, rank: 1 }],
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

function changeableMediaQuery(initialMatches: boolean): MediaQueryList & { setMatches: (value: boolean) => void } {
  let listener: ((event: MediaQueryListEvent) => void) | undefined
  const query = {
    matches: initialMatches,
    media: "(min-width: 1280px)",
    onchange: null,
    addEventListener: vi.fn((_type: string, callback: EventListenerOrEventListenerObject) => { if (typeof callback === "function") listener = callback as (event: MediaQueryListEvent) => void }),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
    setMatches(value: boolean): void { query.matches = value; listener?.({ matches: value } as MediaQueryListEvent) },
  } as MediaQueryList & { matches: boolean; setMatches: (value: boolean) => void }
  return query
}

beforeEach(() => {
  networkLifecycle.mounts = 0
  networkLifecycle.unmounts = 0
  defaultExplorerProps.onClearFocus.mockReset()
  vi.mocked(getGraphSummary).mockReset().mockResolvedValue(summary)
  vi.mocked(getGraphOverview).mockReset().mockResolvedValue(projection("overview"))
  vi.mocked(getGraphSubgraph).mockReset()
  vi.mocked(getEntity).mockReset()
  vi.mocked(getRelationship).mockReset()
  vi.mocked(getCommunity).mockReset()
  vi.mocked(getCommunityReport).mockReset()
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

describe("GraphExplorer focus flow", () => {
  it("opens candidate detail without changing the projection or requesting a subgraph", async () => {
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: "150", title: "Alice", entity_type: "PERSON", degree: 2, rank: 1,
      description: "Candidate detail", community_ids: [], text_unit_ids: [],
    })
    const inspectIntent: GraphInspectIntent = {
      revision: 1,
      candidate: { stableId: "entity-1", shortId: "150", title: "Alice", recordType: "entity", score: 0.887, rank: 2, selected: true, reason: "ann_result", selectionStatus: "selected", finalContext: "included" },
    }

    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} inspectIntent={inspectIntent} />)

    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    expect(await screen.findByText("Candidate detail")).toBeInTheDocument()
    expect(screen.getByRole("region", { name: "Query decision" })).toHaveTextContent("Final contextIncluded")
    expect(screen.getByText("overview")).toBeInTheDocument()
    expect(getEntity).toHaveBeenCalledWith("entity-1", expect.any(AbortSignal))
    expect(getGraphSubgraph).not.toHaveBeenCalled()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(networkLifecycle.mounts).toBe(1)
  })

  it("opens relationship candidates with the same inspect-only semantics", async () => {
    vi.mocked(getRelationship).mockResolvedValue({
      id: "relationship-1", short_id: "23", source: "Alice", target: "Bob", weight: 0.8, rank: 2,
      description: "Candidate relationship", text_unit_ids: [],
    })
    const inspectIntent: GraphInspectIntent = {
      revision: 1,
      candidate: { stableId: "relationship-1", shortId: "23", title: "Alice knows Bob", recordType: "relationship", score: 0.75, selected: true, selectionStatus: "selected", finalContext: "included" },
    }

    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} inspectIntent={inspectIntent} />)

    expect(await screen.findByText("Candidate relationship")).toBeInTheDocument()
    expect(screen.getByText("OVERVIEW")).toBeInTheDocument()
    expect(screen.getByText("overview")).toBeInTheDocument()
    expect(getRelationship).toHaveBeenCalledWith("relationship-1", expect.any(AbortSignal))
    expect(getGraphSubgraph).not.toHaveBeenCalled()
  })

  it("clears candidate-owned detail on Run switch and accepts a reused revision", async () => {
    const detail = (id: string, title: string): GraphEntityDetail => ({
      id, short_id: null, title, entity_type: "PERSON", degree: 1, rank: 1,
      description: `${title} detail`, community_ids: [], text_unit_ids: [],
    })
    vi.mocked(getEntity)
      .mockResolvedValueOnce(detail("entity-a", "Candidate A"))
      .mockResolvedValueOnce(detail("entity-b", "Candidate B"))
    const candidateA: GraphInspectIntent = {
      revision: 1,
      candidate: { stableId: "entity-a", title: "Candidate A", recordType: "entity", selected: true, selectionStatus: "selected", finalContext: "included" },
    }
    const candidateB: GraphInspectIntent = {
      revision: 1,
      candidate: { stableId: "entity-b", title: "Candidate B", recordType: "entity", selected: true, selectionStatus: "selected", finalContext: "included" },
    }
    const { rerender } = render(<GraphExplorer runId="run-a" focusIntent={null} inspectIntent={candidateA} onClearFocus={defaultExplorerProps.onClearFocus} />)
    expect(await screen.findByText("Candidate A detail")).toBeInTheDocument()
    expect(screen.getByRole("region", { name: "Query decision" })).toBeInTheDocument()

    rerender(<GraphExplorer runId="run-b" focusIntent={null} inspectIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)
    await waitFor(() => expect(screen.queryByText("Candidate A detail")).not.toBeInTheDocument())
    expect(screen.queryByRole("region", { name: "Query decision" })).not.toBeInTheDocument()

    rerender(<GraphExplorer runId="run-b" focusIntent={null} inspectIntent={candidateB} onClearFocus={defaultExplorerProps.onClearFocus} />)
    expect(await screen.findByText("Candidate B detail")).toBeInTheDocument()
    expect(getEntity).toHaveBeenCalledTimes(2)
    expect(getGraphSubgraph).not.toHaveBeenCalled()
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("aborts and ignores a pending candidate detail when the Run changes", async () => {
    let resolveCandidate: ((value: GraphEntityDetail) => void) | undefined
    vi.mocked(getEntity).mockReturnValue(new Promise((resolve) => { resolveCandidate = resolve }))
    const inspectIntent: GraphInspectIntent = {
      revision: 1,
      candidate: { stableId: "entity-a", title: "Candidate A", recordType: "entity", selected: true, selectionStatus: "selected", finalContext: "included" },
    }
    const { rerender } = render(<GraphExplorer runId="run-a" focusIntent={null} inspectIntent={inspectIntent} onClearFocus={defaultExplorerProps.onClearFocus} />)
    await waitFor(() => expect(getEntity).toHaveBeenCalledOnce())
    const signal = vi.mocked(getEntity).mock.calls[0]?.[1]

    rerender(<GraphExplorer runId="run-b" focusIntent={null} inspectIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)
    await waitFor(() => expect(signal?.aborted).toBe(true))
    expect(screen.queryByText("Loading structured detail…")).not.toBeInTheDocument()

    await act(async () => {
      resolveCandidate?.({
        id: "entity-a", short_id: null, title: "Stale candidate A", entity_type: "PERSON", degree: 1, rank: 1,
        description: "Stale detail", community_ids: [], text_unit_ids: [],
      })
      await Promise.resolve()
    })
    expect(screen.queryByText("Stale detail")).not.toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Query decision" })).not.toBeInTheDocument()
    expect(getGraphSubgraph).not.toHaveBeenCalled()
    expect(screen.getByText("overview")).toBeInTheDocument()
  })

  it("preserves ordinary graph inspection while clearing only Run-owned decisions", async () => {
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: null, title: "Graph selection", entity_type: "PERSON", degree: 1, rank: 1,
      description: "Ordinary graph detail", community_ids: [], text_unit_ids: [],
    })
    const user = userEvent.setup()
    const { rerender } = render(<GraphExplorer runId="run-a" focusIntent={null} inspectIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)
    await user.click(await screen.findByRole("button", { name: "Open entity test" }))
    expect(await screen.findByText("Ordinary graph detail")).toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Query decision" })).not.toBeInTheDocument()

    rerender(<GraphExplorer runId="run-b" focusIntent={null} inspectIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)

    expect(screen.getByText("Ordinary graph detail")).toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Query decision" })).not.toBeInTheDocument()
    expect(getGraphSubgraph).not.toHaveBeenCalled()
  })

  it("does not load a subgraph for citation emphasis and clears it only when a projection commits", async () => {
    const onClearEmphasis = vi.fn()
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} emphasisIntent={{ entityIds: ["entity-1"], relationshipIds: [], revision: 1 }} onClearEmphasis={onClearEmphasis} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    expect(onClearEmphasis).toHaveBeenCalledOnce()
    expect(getGraphSubgraph).not.toHaveBeenCalled()

    rerender(<GraphExplorer {...defaultExplorerProps} focusIntent={null} emphasisIntent={{ entityIds: [], relationshipIds: ["relationship-1"], revision: 2 }} onClearEmphasis={onClearEmphasis} />)
    await Promise.resolve()
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(getGraphSubgraph).not.toHaveBeenCalled()
    expect(onClearEmphasis).toHaveBeenCalledOnce()
  })

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
      degree: 1,
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
      degree: 1,
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
    await user.click(screen.getByRole("button", { name: "Clear graph selection" }))

    await user.click(screen.getByRole("button", { name: "Back test" }))

    expect(await screen.findByText("OVERVIEW-NEW")).toBeInTheDocument()
    expect(defaultExplorerProps.onClearFocus).not.toHaveBeenCalled()
    expect(getGraphOverview).toHaveBeenCalledTimes(2)
  })

  it("restores Query focus after exploring an entity neighborhood", async () => {
    const queryIntent: GraphFocusIntent = { entity_ids: ["query-seed"], relationship_ids: [], depth: 1, max_entities: 80, max_relationships: 160, revision: 1 }
    vi.mocked(getGraphSubgraph)
      .mockResolvedValueOnce(projection("query-focus", true))
      .mockResolvedValueOnce(projection("entity-focus", true))
      .mockResolvedValueOnce(projection("query-restored", true))
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 1, rank: 1,
      description: "Query entity", community_ids: [], text_unit_ids: [],
    })
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={queryIntent} />)
    expect(await screen.findByText("QUERY-FOCUS")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    await user.click(await screen.findByRole("button", { name: "Focus neighborhood" }))
    expect(await screen.findByText("ENTITY-FOCUS")).toBeInTheDocument()
    expect(screen.getByText("Back to query focus")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Back test" }))

    expect(await screen.findByText("QUERY-RESTORED")).toBeInTheDocument()
    expect(screen.getByText("query-focus")).toBeInTheDocument()
    expect(vi.mocked(getGraphSubgraph).mock.calls[2]?.[0]).toEqual({ entity_ids: ["query-seed"], relationship_ids: [], depth: 1, max_entities: 80, max_relationships: 160 })
    expect(defaultExplorerProps.onClearFocus).not.toHaveBeenCalled()
  })

  it("invalidates a Query navigation origin when the Run changes and aborts stale restoration", async () => {
    let resolveRestore: ((value: GraphProjection) => void) | undefined
    const restore = new Promise<GraphProjection>((resolve) => { resolveRestore = resolve })
    const queryIntent: GraphFocusIntent = { entity_ids: ["query-seed"], relationship_ids: [], depth: 1, max_entities: 80, max_relationships: 160, revision: 1 }
    vi.mocked(getGraphSubgraph)
      .mockResolvedValueOnce(projection("query-focus", true))
      .mockResolvedValueOnce(projection("entity-focus", true))
      .mockReturnValueOnce(restore)
    vi.mocked(getGraphOverview).mockResolvedValue(projection("run-b-overview"))
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 1, rank: 1,
      description: "Query entity", community_ids: [], text_unit_ids: [],
    })
    const user = userEvent.setup()
    const { rerender } = render(<GraphExplorer runId="run-a" focusIntent={queryIntent} onClearFocus={defaultExplorerProps.onClearFocus} />)
    expect(await screen.findByText("QUERY-FOCUS")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    await user.click(await screen.findByRole("button", { name: "Focus neighborhood" }))
    expect(await screen.findByText("ENTITY-FOCUS")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Back test" }))
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(3))
    const restoreSignal = vi.mocked(getGraphSubgraph).mock.calls[2]?.[1]

    rerender(<GraphExplorer runId="run-b" focusIntent={null} onClearFocus={defaultExplorerProps.onClearFocus} />)

    expect(await screen.findByText("RUN-B-OVERVIEW")).toBeInTheDocument()
    expect(restoreSignal?.aborted).toBe(true)
    resolveRestore?.(projection("stale-query", true))
    await Promise.resolve()
    expect(screen.queryByText("STALE-QUERY")).not.toBeInTheDocument()
    expect(screen.queryByText("Back to query focus")).not.toBeInTheDocument()
  })

  it("aborts pending Query restoration when a New Query resets navigation", async () => {
    let resolveRestore: ((value: GraphProjection) => void) | undefined
    const restore = new Promise<GraphProjection>((resolve) => { resolveRestore = resolve })
    const queryIntent: GraphFocusIntent = { entity_ids: ["query-seed"], relationship_ids: [], depth: 1, max_entities: 80, max_relationships: 160, revision: 1 }
    vi.mocked(getGraphSubgraph)
      .mockResolvedValueOnce(projection("query-focus", true))
      .mockResolvedValueOnce(projection("entity-focus", true))
      .mockReturnValueOnce(restore)
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 1, rank: 1,
      description: "Query entity", community_ids: [], text_unit_ids: [],
    })
    const user = userEvent.setup()
    const { rerender } = render(<GraphExplorer {...defaultExplorerProps} focusIntent={queryIntent} navigationResetRevision={0} />)
    expect(await screen.findByText("QUERY-FOCUS")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    await user.click(await screen.findByRole("button", { name: "Focus neighborhood" }))
    expect(await screen.findByText("ENTITY-FOCUS")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Back test" }))
    await waitFor(() => expect(getGraphSubgraph).toHaveBeenCalledTimes(3))
    const restoreSignal = vi.mocked(getGraphSubgraph).mock.calls[2]?.[1]

    rerender(<GraphExplorer {...defaultExplorerProps} focusIntent={queryIntent} navigationResetRevision={1} />)

    expect(restoreSignal?.aborted).toBe(true)
    resolveRestore?.(projection("stale-query", true))
    await Promise.resolve()
    expect(screen.queryByText("STALE-QUERY")).not.toBeInTheDocument()
    expect(screen.getByText("ENTITY-FOCUS")).toBeInTheDocument()
    expect(screen.queryByText("Back to query focus")).not.toBeInTheDocument()
  })

  it("keeps Inspector selection mounted while the right panel is collapsed", async () => {
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 1, rank: 1,
      description: "Selected entity", community_ids: [], text_unit_ids: [],
    })
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    expect(await screen.findByText("Selected entity")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Collapse graph inspector" }))
    expect(screen.getByText("Selected entity")).toBeInTheDocument()
  })

  it("opens mobile Inspector detail without unmounting the focused graph", async () => {
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false, media: "(min-width: 1280px)", onchange: null,
      addEventListener: vi.fn(), removeEventListener: vi.fn(), addListener: vi.fn(), removeListener: vi.fn(), dispatchEvent: vi.fn(),
    })))
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 1, rank: 1,
      description: "Mobile detail", community_ids: [], text_unit_ids: [],
    })
    const user = userEvent.setup()
    const intent: GraphFocusIntent = { entity_ids: ["entity-1"], relationship_ids: [], revision: 1 }
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={intent} />)
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()
    expect(screen.getByText("query-focus")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    expect(await screen.findByText("Mobile detail")).toBeInTheDocument()
    expect(screen.getByRole("tab", { name: "Detail" })).toHaveAttribute("aria-selected", "true")
    expect(screen.getByText("FOCUSED")).toBeInTheDocument()
    expect(getGraphSubgraph).toHaveBeenCalledTimes(1)
  })

  it("keeps one NetworkPreview owner, requests, and Inspector selection across breakpoints and narrow tabs", async () => {
    const media = changeableMediaQuery(true)
    vi.stubGlobal("matchMedia", vi.fn(() => media))
    vi.mocked(getEntity).mockResolvedValue({
      id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 1, rank: 1,
      description: "Persistent Inspector detail", community_ids: [], text_unit_ids: [],
    })
    const user = userEvent.setup()
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={null} />)
    expect(await screen.findByText("OVERVIEW")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open entity test" }))
    expect(await screen.findByText("Persistent Inspector detail")).toBeInTheDocument()

    act(() => media.setMatches(false))
    expect(screen.getByRole("tab", { name: "Detail" })).toHaveAttribute("aria-selected", "true")
    await user.click(screen.getByRole("tab", { name: "Graph" }))
    await user.click(screen.getByRole("tab", { name: "Detail" }))
    act(() => media.setMatches(true))

    expect(screen.getByText("Persistent Inspector detail")).toBeInTheDocument()
    expect(networkLifecycle.mounts).toBe(1)
    expect(networkLifecycle.unmounts).toBe(0)
    expect(getGraphSummary).toHaveBeenCalledTimes(1)
    expect(getGraphOverview).toHaveBeenCalledTimes(1)
    expect(getGraphSubgraph).not.toHaveBeenCalled()
    expect(getEntity).toHaveBeenCalledTimes(1)
  })

  it("keeps the initial Query focus owner and request stable across breakpoints", async () => {
    const media = changeableMediaQuery(true)
    vi.stubGlobal("matchMedia", vi.fn(() => media))
    vi.mocked(getGraphSubgraph).mockResolvedValue(projection("focused", true))
    const intent: GraphFocusIntent = { entity_ids: ["entity-1"], relationship_ids: [], revision: 1 }
    render(<GraphExplorer {...defaultExplorerProps} focusIntent={intent} />)
    expect(await screen.findByText("FOCUSED")).toBeInTheDocument()

    act(() => media.setMatches(false))
    act(() => media.setMatches(true))

    expect(networkLifecycle.mounts).toBe(1)
    expect(networkLifecycle.unmounts).toBe(0)
    expect(getGraphSummary).toHaveBeenCalledTimes(1)
    expect(getGraphSubgraph).toHaveBeenCalledTimes(1)
    expect(getGraphOverview).not.toHaveBeenCalled()
    expect(screen.getByText("query-focus")).toBeInTheDocument()
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
