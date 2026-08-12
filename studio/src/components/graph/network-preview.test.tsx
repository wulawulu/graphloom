import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { GraphProjection } from "@/api/types"
import { NetworkPreview } from "@/components/graph/network-preview"
import { GRAPH_LAYOUT_OPTIONS } from "@/lib/graph"

const rendererTheme = {
  "--cy-background": "#080a0e",
  "--cy-foreground": "#ebeff5",
  "--cy-border": "#282b33",
  "--cy-graph-person": "#00bdce",
  "--cy-graph-organization": "#bc90ee",
  "--cy-graph-geo": "#5bbd74",
  "--cy-graph-event": "#eba941",
  "--cy-graph-default": "#7d8a9f",
  "--cy-graph-seed": "#f9bd01",
}

interface CytoscapeTestEvent {
  target: { id: () => string; addClass: (value: string) => void; removeClass: (value: string) => void }
  renderedPosition?: { x: number; y: number }
}

const graphMock = vi.hoisted(() => ({
  handlers: new Map<string, (event: CytoscapeTestEvent) => void>(),
  instances: [] as unknown[],
  observers: [] as ResizeObserverCallback[],
  layoutCalls: 0,
  fitCalls: 0,
  width: 640,
  height: 480,
}))

vi.mock("cytoscape", () => ({
  default: vi.fn(() => {
    let onLayoutStop: (() => void) | undefined
    const instance = {
      on: vi.fn((event: string, selector: string, handler: (value: CytoscapeTestEvent) => void) => graphMock.handlers.set(`${event}:${selector}`, handler)),
      elements: vi.fn(() => ({ filter: vi.fn(() => ({ length: 1 })) })),
      layout: vi.fn(() => {
        graphMock.layoutCalls += 1
        return { one: vi.fn((_event: string, handler: () => void) => { onLayoutStop = handler }), run: vi.fn(() => onLayoutStop?.()) }
      }),
      animate: vi.fn(), fit: vi.fn(() => { graphMock.fitCalls += 1 }), resize: vi.fn(), destroy: vi.fn(),
    }
    graphMock.instances.push(instance)
    return instance
  }),
}))

const projection: GraphProjection = {
  entities: [{ id: "entity-1", title: "Alice", entity_type: "PERSON", degree: 1, rank: 1 }],
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
  graphMock.instances.length = 0
  graphMock.observers.length = 0
  graphMock.layoutCalls = 0
  graphMock.fitCalls = 0
  graphMock.width = 640
  graphMock.height = 480
  for (const [name, value] of Object.entries(rendererTheme)) {
    document.documentElement.style.setProperty(name, value)
  }
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(() => graphMock.width)
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(() => graphMock.height)
  vi.stubGlobal("ResizeObserver", class {
    constructor(private readonly callback: ResizeObserverCallback) { graphMock.observers.push(callback) }
    observe(): void { this.callback([], this) }
    unobserve(): void {}
    disconnect(): void {}
  })
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  for (const name of Object.keys(rendererTheme)) document.documentElement.style.removeProperty(name)
})

describe("NetworkPreview focus labels", () => {
  it("separates the layout wrapper from the full-size Cytoscape container", () => {
    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)

    const container = screen.getByLabelText("Knowledge graph network preview")
    expect(container).toHaveClass("size-full")
    expect(container).not.toHaveClass("absolute", "inset-0")
    expect(container.parentElement).toHaveClass("absolute", "inset-0")
    expect(container.parentElement).not.toBe(container)
  })

  it("waits for a positive height before initializing Cytoscape exactly once", async () => {
    const { default: cytoscape } = await import("cytoscape")
    const initialConstructorCalls = vi.mocked(cytoscape).mock.calls.length
    graphMock.width = 1_396
    graphMock.height = 0

    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)

    expect(cytoscape).toHaveBeenCalledTimes(initialConstructorCalls)
    expect(graphMock.instances).toHaveLength(0)
    expect(graphMock.layoutCalls).toBe(0)
    expect(graphMock.fitCalls).toBe(0)

    graphMock.height = 474
    act(() => graphMock.observers[0]?.([], {} as ResizeObserver))

    await waitFor(() => expect(cytoscape).toHaveBeenCalledTimes(initialConstructorCalls + 1))
    expect(graphMock.instances).toHaveLength(1)
    const instance = graphMock.instances[0] as {
      layout: ReturnType<typeof vi.fn>
      fit: ReturnType<typeof vi.fn>
    }
    expect(instance.layout).toHaveBeenCalledOnce()
    expect(instance.fit).toHaveBeenCalledOnce()

    act(() => graphMock.observers[0]?.([], {} as ResizeObserver))
    expect(cytoscape).toHaveBeenCalledTimes(initialConstructorCalls + 1)
  })

  it("initializes Cytoscape with supported colors and deterministic label width", async () => {
    const { default: cytoscape } = await import("cytoscape")
    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)

    await waitFor(() => expect(cytoscape).toHaveBeenCalled())
    const options = vi.mocked(cytoscape).mock.calls.at(-1)?.[0] as unknown as {
      elements?: unknown
      style?: unknown
      userZoomingEnabled?: boolean
      userPanningEnabled?: boolean
    }
    const serializedStyles = JSON.stringify(options?.style)
    expect(serializedStyles).not.toContain("oklch")
    expect(serializedStyles).not.toContain('"width":"label"')
    for (const color of Object.values(rendererTheme)) expect(serializedStyles).toContain(color)
    expect(options?.elements).toEqual(expect.arrayContaining([
      expect.objectContaining({ data: expect.objectContaining({ displayWidth: expect.any(Number), displayHeight: expect.any(Number), label: "Alice" }) }),
    ]))
    expect(serializedStyles).toContain('"text-halign":"center"')
    expect(serializedStyles).toContain('"text-valign":"center"')
    expect(serializedStyles).toContain('"label":"data(label)"')
    expect(options?.userZoomingEnabled).toBe(true)
    expect(options?.userPanningEnabled).toBe(true)
  })

  it("uses the expanded topology layout parameters", () => {
    expect(GRAPH_LAYOUT_OPTIONS.nodeRepulsion()).toBe(22_000)
    expect(GRAPH_LAYOUT_OPTIONS.nodeOverlap).toBe(40)
    expect(GRAPH_LAYOUT_OPTIONS.idealEdgeLength()).toBe(130)
    expect(GRAPH_LAYOUT_OPTIONS.componentSpacing).toBe(120)
    expect(GRAPH_LAYOUT_OPTIONS.padding).toBe(64)
  })

  it("shows a safe error and logs diagnostics when a renderer token is invalid", async () => {
    const diagnostic = vi.spyOn(console, "error").mockImplementation(() => undefined)
    document.documentElement.style.setProperty("--cy-graph-person", "oklch(0.72 0.14 205)")
    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)

    expect(await screen.findByRole("alert")).toHaveTextContent("Graph visualization failed to initialize.")
    expect(screen.getByRole("alert")).toHaveTextContent("Check the browser console for renderer diagnostics.")
    expect(diagnostic).toHaveBeenCalledWith(
      "Graph visualization failed to initialize",
      expect.any(Error),
    )
  })

  it("shows a safe error and logs diagnostics when Cytoscape construction throws", async () => {
    const { default: cytoscape } = await import("cytoscape")
    const diagnostic = vi.spyOn(console, "error").mockImplementation(() => undefined)
    vi.mocked(cytoscape).mockImplementationOnce(() => { throw new Error("renderer fixture") })
    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)

    expect(await screen.findByRole("alert")).toHaveTextContent("Graph visualization failed to initialize.")
    expect(diagnostic).toHaveBeenCalledWith(
      "Graph visualization failed to initialize",
      expect.objectContaining({ message: "renderer fixture" }),
    )
  })

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
    expect(screen.getByRole("tooltip")).toHaveTextContent("Degree 1")
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

  it("resizes without fitting or laying out when a persistent canvas becomes visible", async () => {
    render(<NetworkPreview {...callbacks} projection={projection} summary={null} summaryError={false} mode="overview" loading={false} error={null} />)
    await waitFor(() => expect(graphMock.instances).toHaveLength(1))
    const instance = graphMock.instances[0] as {
      layout: ReturnType<typeof vi.fn>
      fit: ReturnType<typeof vi.fn>
      resize: ReturnType<typeof vi.fn>
    }
    const layoutCalls = instance.layout.mock.calls.length
    const fitCalls = instance.fit.mock.calls.length

    graphMock.width = 0
    act(() => graphMock.observers[0]?.([], {} as ResizeObserver))
    graphMock.width = 640
    act(() => graphMock.observers[0]?.([], {} as ResizeObserver))

    expect(instance.resize).toHaveBeenCalledOnce()
    expect(instance.layout).toHaveBeenCalledTimes(layoutCalls)
    expect(instance.fit).toHaveBeenCalledTimes(fitCalls)
  })
})
