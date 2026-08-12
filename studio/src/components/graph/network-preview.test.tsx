import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { GraphProjection } from "@/api/types"
import { NetworkPreview } from "@/components/graph/network-preview"

interface CytoscapeTestEvent {
  target: { id: () => string; addClass: (value: string) => void; removeClass: (value: string) => void }
  renderedPosition?: { x: number; y: number }
}

const graphMock = vi.hoisted(() => ({ handlers: new Map<string, (event: CytoscapeTestEvent) => void>() }))

vi.mock("cytoscape", () => ({
  default: vi.fn(() => {
    let onLayoutStop: (() => void) | undefined
    return {
      on: vi.fn((event: string, selector: string, handler: (value: CytoscapeTestEvent) => void) => graphMock.handlers.set(`${event}:${selector}`, handler)),
      elements: vi.fn(() => ({ filter: vi.fn(() => ({ length: 1 })) })),
      layout: vi.fn(() => ({ one: vi.fn((_event: string, handler: () => void) => { onLayoutStop = handler }), run: vi.fn(() => onLayoutStop?.()) })),
      animate: vi.fn(), fit: vi.fn(), resize: vi.fn(), destroy: vi.fn(),
    }
  }),
}))

const projection: GraphProjection = {
  entities: [{ id: "entity-1", title: "Alice", entity_type: "PERSON", rank: 1 }],
  relationships: [],
  seed_entity_ids: ["entity-1"],
  seed_relationship_ids: [],
  missing_entity_ids: [],
  missing_relationship_ids: [],
  unresolved_relationship_ids: [],
  unresolved_relationship_count: 0,
  truncated: false,
}

const callbacks = {
  onEntity: vi.fn(),
  onRelationship: vi.fn(),
  onBackOverview: vi.fn(),
  onReload: vi.fn(),
}

beforeEach(() => {
  graphMock.handlers.clear()
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(640)
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(480)
  vi.stubGlobal("ResizeObserver", class {
    constructor(private readonly callback: ResizeObserverCallback) {}
    observe(): void { this.callback([], this) }
    unobserve(): void {}
    disconnect(): void {}
  })
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("NetworkPreview focus labels", () => {
  it("labels Timeline focus as Query focus with a Query seed", () => {
    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="query-focus" loading={false} error={null} />)

    expect(screen.getByText("Query focus")).toBeInTheDocument()
    expect(screen.getByText("Query seed")).toBeInTheDocument()
    expect(screen.queryByText("Focused subgraph")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Back to overview" })).toBeInTheDocument()
  })

  it("labels manual focus as a Focused subgraph with a generic Seed", () => {
    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="explorer-focus" loading={false} error={null} />)

    expect(screen.getByText("Focused subgraph")).toBeInTheDocument()
    expect(screen.getByText("Seed")).toBeInTheDocument()
    expect(screen.queryByText("Query focus")).not.toBeInTheDocument()
    expect(screen.queryByText("Query seed")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Back to overview" })).toBeInTheDocument()
  })

  it("shows and removes projection tooltips while clicks request Inspector detail", async () => {
    const relationshipProjection: GraphProjection = {
      ...projection,
      relationships: [{ id: "relationship-1", source_entity_id: "entity-1", target_entity_id: "entity-1", source: "Alice", target: "Alice", weight: 3.2, rank: 8 }],
    }
    render(<NetworkPreview {...callbacks} projection={relationshipProjection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)
    await waitFor(() => expect(graphMock.handlers.has("mouseover:node")).toBe(true))
    const target = { id: () => "entity-1", addClass: vi.fn(), removeClass: vi.fn() }

    act(() => graphMock.handlers.get("mouseover:node")?.({ target, renderedPosition: { x: 30, y: 40 } }))
    expect(screen.getByRole("tooltip")).toHaveTextContent("Alice")
    act(() => graphMock.handlers.get("mouseout:node")?.({ target }))
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument()

    act(() => graphMock.handlers.get("mouseover:edge")?.({ target: { ...target, id: () => "relationship-1" }, renderedPosition: { x: 50, y: 60 } }))
    expect(screen.getByRole("tooltip")).toHaveTextContent("Weight 3.2")
    act(() => graphMock.handlers.get("tap:node")?.({ target }))
    expect(callbacks.onEntity).toHaveBeenCalledWith("entity-1")
  })

  it("keeps the Cytoscape owner when click callback identities change", async () => {
    const { default: cytoscape } = await import("cytoscape")
    const initialCalls = vi.mocked(cytoscape).mock.calls.length
    const { rerender } = render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)
    await waitFor(() => expect(cytoscape).toHaveBeenCalledTimes(initialCalls + 1))

    rerender(<NetworkPreview {...callbacks} onEntity={vi.fn()} onRelationship={vi.fn()} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)
    expect(cytoscape).toHaveBeenCalledTimes(initialCalls + 1)
  })
})
