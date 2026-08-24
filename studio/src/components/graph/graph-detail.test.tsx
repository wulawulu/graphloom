import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { GraphCommunity, GraphCommunityReportDetail, GraphEntityDetail, GraphRelationshipDetail } from "@/api/types"
import { GraphInspector } from "@/components/graph/graph-detail"

afterEach(cleanup)

const common = { loading: false, error: false, canGoBack: false, onBack: vi.fn(), onClear: vi.fn(), onOpenEntity: vi.fn(), onOpenCommunity: vi.fn(), onFocusEntity: vi.fn(), onFocusRelationship: vi.fn() }

describe("GraphInspector", () => {
  it("renders structured entity detail, collapses long sources, and focuses explicitly", async () => {
    const user = userEvent.setup()
    const onFocusEntity = vi.fn()
    const entity: GraphEntityDetail = { id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 12, rank: 12, description: "Alice description", community_ids: ["5"], communities: [{ id: "community-5", short_id: "5", title: "Alice network", report_title: "Alice semantic network", level: 1, summary: "People connected to Alice" }], text_unit_ids: Array.from({ length: 25 }, (_, index) => `text-${index + 1}`) }
    render(<GraphInspector {...common} onFocusEntity={onFocusEntity} detail={{ kind: "entity", value: entity }} />)

    expect(screen.getByText("PERSON")).toBeInTheDocument()
    expect(screen.getByText("Degree 12")).toBeInTheDocument()
    expect(screen.getByText("Rank 12")).toBeInTheDocument()
    expect(screen.getByText("Alice description")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Open community Alice semantic network" })).toBeInTheDocument()
    expect(screen.getByText("Developer · Raw JSON")).toBeInTheDocument()
    expect(screen.getByText("25 source text units")).toBeInTheDocument()
    expect(screen.queryByText("text-25")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Show 5 more" }))
    expect(screen.getByText("text-25")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Focus neighborhood" }))
    expect(onFocusEntity).toHaveBeenCalledWith("entity-1")
  })

  it("renders relationship direction and focus action", async () => {
    const user = userEvent.setup()
    const onFocusRelationship = vi.fn()
    const onOpenEntity = vi.fn()
    const relationship: GraphRelationshipDetail = { id: "relationship-1", short_id: null, source: "Alice", target: "Acme", source_entity: { id: "entity-alice", short_id: "E1", title: "Alice", entity_type: "PERSON", description: "Alice preview" }, target_entity: { id: "entity-acme", short_id: "E2", title: "Acme", entity_type: "ORGANIZATION", description: null }, weight: 0.8, rank: 4, description: "works at", text_unit_ids: [] }
    render(<GraphInspector {...common} onOpenEntity={onOpenEntity} onFocusRelationship={onFocusRelationship} detail={{ kind: "relationship", value: relationship }} />)

    expect(screen.getByRole("button", { name: "Open entity Alice" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Open entity Acme" })).toBeInTheDocument()
    expect(screen.getByText("Weight 0.8")).toBeInTheDocument()
    await user.hover(screen.getByRole("button", { name: "Open entity Alice" }))
    expect(await screen.findByText("Alice preview")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open entity Alice" }))
    await user.click(screen.getByRole("button", { name: "Open entity Acme" }))
    expect(onOpenEntity).toHaveBeenNthCalledWith(1, "entity-alice")
    expect(onOpenEntity).toHaveBeenNthCalledWith(2, "entity-acme")
    expect(onFocusRelationship).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Focus relationship" }))
    expect(onFocusRelationship).toHaveBeenCalledWith("relationship-1")
  })

  it("combines graph detail with the originating query decision", () => {
    const entity: GraphEntityDetail = { id: "entity-1", short_id: "150", title: "Alice", entity_type: "PERSON", degree: 2, rank: 1, description: null, community_ids: [], communities: [], text_unit_ids: [] }
    render(<GraphInspector {...common} detail={{ kind: "entity", value: entity }} decision={{ stableId: "entity-1", shortId: "150", title: "Alice", recordType: "entity", score: 0.887, rank: 2, selected: true, reason: "ann_result", selectionStatus: "selected", finalContext: "excluded" }} />)

    expect(screen.getByRole("region", { name: "Query decision" })).toHaveTextContent("Retrieval score0.8870")
    expect(screen.getByRole("region", { name: "Query decision" })).toHaveTextContent("Retrieval rank2")
    expect(screen.getByRole("region", { name: "Query decision" })).toHaveTextContent("SelectionSelected")
    expect(screen.getByRole("region", { name: "Query decision" })).toHaveTextContent("Final contextNot included")
    expect(screen.getByRole("region", { name: "Query decision" })).toHaveTextContent("ann result")
  })

  it("renders community hierarchy and a safe formatted report", () => {
    const community: GraphCommunity = { id: "community-1", short_id: "5", title: "Alice network", level: 1, parent: 0, children: [6], parent_community: { id: "community-0", short_id: "0", title: "Root network", report_title: "Root semantic network", level: 2, summary: "Root summary" }, child_communities: [{ id: "community-6", short_id: "6", title: "Child network", report_title: "Child semantic network", level: 0, summary: null }], report: { id: "report-1", short_id: "5", community_id: "5", title: "Network report", summary: "A useful summary", rank: 3 } }
    const report: GraphCommunityReportDetail = { ...community.report!, full_content: "## Report heading\n\n- first point\n\n![remote](https://example.com/image.png)" }
    render(<GraphInspector {...common} detail={{ kind: "community", value: community, report }} />)

    expect(screen.getByText("L1")).toBeInTheDocument()
    expect(screen.getByText("A useful summary")).toBeInTheDocument()
    expect(screen.getByRole("heading", { level: 2, name: "Network report" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Open community Root semantic network" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Open community Child semantic network" })).toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "Report heading" })).toBeInTheDocument()
    expect(screen.getByText("[Remote image omitted: remote]")).toBeInTheDocument()
  })

  it("renders root and leaf states and progressively reveals large child hierarchies", async () => {
    const user = userEvent.setup()
    const children = Array.from({ length: 11 }, (_, index) => ({ id: `community-${index}`, short_id: String(index), title: `Child ${index}`, report_title: null, level: 0, summary: null }))
    const community: GraphCommunity = { id: "root", short_id: "99", title: "Root", level: 1, parent: -1, children: children.map((child) => Number(child.short_id)), parent_community: null, child_communities: children, report: null }
    render(<GraphInspector {...common} detail={{ kind: "community", value: community, report: null }} />)

    expect(screen.getByText("Root community · no parent")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Open community Child 10" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Show all children" }))
    expect(screen.getByRole("button", { name: "Open community Child 10" })).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Show fewer children" }))
    expect(screen.queryByRole("button", { name: "Open community Child 10" })).not.toBeInTheDocument()
  })

  it("resets child expansion when navigating to another community", async () => {
    const user = userEvent.setup()
    const community = (id: string, prefix: string): GraphCommunity => {
      const children = Array.from({ length: 11 }, (_, index) => ({ id: `${id}-child-${index}`, short_id: String(index), title: `${prefix} Child ${index}`, report_title: null, level: 0, summary: null }))
      return { id, short_id: id, title: prefix, level: 1, parent: -1, children: children.map((child) => Number(child.short_id)), parent_community: null, child_communities: children, report: null }
    }
    const first = community("first", "First")
    const second = community("second", "Second")
    const { rerender } = render(<GraphInspector {...common} detail={{ kind: "community", value: first, report: null }} />)
    await user.click(screen.getByRole("button", { name: "Show all children" }))
    expect(screen.getByRole("button", { name: "Open community First Child 10" })).toBeInTheDocument()

    rerender(<GraphInspector {...common} detail={{ kind: "community", value: second, report: null }} />)

    expect(screen.queryByRole("button", { name: "Open community Second Child 10" })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Show all children" })).toBeInTheDocument()
  })

  it("renders a leaf community without navigation requests", () => {
    const community: GraphCommunity = { id: "leaf", short_id: "7", title: "Leaf", level: 0, parent: -1, children: [], parent_community: null, child_communities: [], report: null }
    render(<GraphInspector {...common} detail={{ kind: "community", value: community, report: null }} />)
    expect(screen.getByRole("heading", { level: 2, name: "Leaf" })).toBeInTheDocument()
    expect(screen.getByText("No child communities")).toBeInTheDocument()
  })

  it("clears the persistent Inspector selection with Escape", async () => {
    const user = userEvent.setup()
    const onClear = vi.fn()
    const entity: GraphEntityDetail = { id: "entity-1", short_id: null, title: "Alice", entity_type: "PERSON", degree: 1, rank: 1, description: null, community_ids: [], communities: [], text_unit_ids: [] }
    render(<GraphInspector {...common} onClear={onClear} detail={{ kind: "entity", value: entity }} />)

    await user.click(screen.getByRole("region", { name: "Graph Inspector" }))
    await user.keyboard("{Escape}")
    expect(onClear).toHaveBeenCalledOnce()
  })

  it("navigates references without invoking graph focus and exposes hover previews", async () => {
    const user = userEvent.setup()
    const onOpenCommunity = vi.fn()
    const onFocusEntity = vi.fn()
    const entity: GraphEntityDetail = { id: "entity-1", short_id: null, title: "Alice", entity_type: "PERSON", degree: 1, rank: 1, description: null, community_ids: ["5", "missing"], communities: [{ id: "community-5", short_id: "5", title: "Alice network", report_title: "Alice semantic network", level: 1, summary: "Preview summary" }], text_unit_ids: [] }
    render(<GraphInspector {...common} onOpenCommunity={onOpenCommunity} onFocusEntity={onFocusEntity} detail={{ kind: "entity", value: entity }} />)

    const community = screen.getByRole("button", { name: "Open community Alice semantic network" })
    await user.hover(community)
    expect(await screen.findByText("Preview summary")).toBeInTheDocument()
    await user.click(community)
    expect(onOpenCommunity).toHaveBeenCalledWith("community-5")
    expect(onFocusEntity).not.toHaveBeenCalled()
    expect(screen.getByText("Unknown community · missing")).toBeInTheDocument()
  })

  it("keeps unresolved relationship endpoints non-clickable and supports Inspector Back", async () => {
    const user = userEvent.setup()
    const onBack = vi.fn()
    const relationship: GraphRelationshipDetail = { id: "relationship-1", short_id: null, source: "Duplicate", target: "Missing", source_entity: null, target_entity: null, weight: null, rank: null, description: null, text_unit_ids: [] }
    render(<GraphInspector {...common} canGoBack onBack={onBack} detail={{ kind: "relationship", value: relationship }} />)

    expect(screen.getAllByText("Unable to uniquely resolve entity")).toHaveLength(2)
    expect(screen.queryByRole("button", { name: /Open entity/ })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Back" }))
    expect(onBack).toHaveBeenCalledOnce()
  })
})
