import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { GraphProjection } from "@/api/types"
import { NetworkPreview } from "@/components/graph/network-preview"

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
  vi.stubGlobal("ResizeObserver", class {
    observe(): void {}
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
    render(<NetworkPreview {...callbacks} projection={projection} mode="query-focus" loading={false} error={null} />)

    expect(screen.getByText("Query focus")).toBeInTheDocument()
    expect(screen.getByText("Query seed")).toBeInTheDocument()
    expect(screen.queryByText("Focused subgraph")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Back to overview" })).toBeInTheDocument()
  })

  it("labels manual focus as a Focused subgraph with a generic Seed", () => {
    render(<NetworkPreview {...callbacks} projection={projection} mode="explorer-focus" loading={false} error={null} />)

    expect(screen.getByText("Focused subgraph")).toBeInTheDocument()
    expect(screen.getByText("Seed")).toBeInTheDocument()
    expect(screen.queryByText("Query focus")).not.toBeInTheDocument()
    expect(screen.queryByText("Query seed")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Back to overview" })).toBeInTheDocument()
  })
})
