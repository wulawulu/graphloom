import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { GraphCommunity, GraphCommunityReportDetail, GraphEntityDetail, GraphRelationshipDetail } from "@/api/types"
import { GraphInspector } from "@/components/graph/graph-detail"

afterEach(cleanup)

const common = { loading: false, error: false, onClear: vi.fn(), onFocusEntity: vi.fn(), onFocusRelationship: vi.fn() }

describe("GraphInspector", () => {
  it("renders structured entity detail, collapses long sources, and focuses explicitly", async () => {
    const user = userEvent.setup()
    const onFocusEntity = vi.fn()
    const entity: GraphEntityDetail = { id: "entity-1", short_id: "E1", title: "Alice", entity_type: "PERSON", degree: 12, rank: 12, description: "Alice description", community_ids: ["5"], text_unit_ids: Array.from({ length: 25 }, (_, index) => `text-${index + 1}`) }
    render(<GraphInspector {...common} onFocusEntity={onFocusEntity} detail={{ kind: "entity", value: entity }} />)

    expect(screen.getByText("PERSON")).toBeInTheDocument()
    expect(screen.getByText("Degree 12")).toBeInTheDocument()
    expect(screen.getByText("Rank 12")).toBeInTheDocument()
    expect(screen.getByText("Alice description")).toBeInTheDocument()
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
    const relationship: GraphRelationshipDetail = { id: "relationship-1", short_id: null, source: "Alice", target: "Acme", weight: 0.8, rank: 4, description: "works at", text_unit_ids: [] }
    render(<GraphInspector {...common} onFocusRelationship={onFocusRelationship} detail={{ kind: "relationship", value: relationship }} />)

    expect(screen.getByText("Alice")).toBeInTheDocument()
    expect(screen.getByText("Acme")).toBeInTheDocument()
    expect(screen.getByText("Weight 0.8")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Focus relationship" }))
    expect(onFocusRelationship).toHaveBeenCalledWith("relationship-1")
  })

  it("renders community hierarchy and a safe formatted report", () => {
    const community: GraphCommunity = { id: "community-1", short_id: "5", title: "Alice network", level: 1, parent: 0, children: [6], report: { id: "report-1", short_id: "5", community_id: "5", title: "Network report", summary: "A useful summary", rank: 3 } }
    const report: GraphCommunityReportDetail = { ...community.report!, full_content: "## Report heading\n\n- first point\n\n![remote](https://example.com/image.png)" }
    render(<GraphInspector {...common} detail={{ kind: "community", value: community, report }} />)

    expect(screen.getByText("Level 1")).toBeInTheDocument()
    expect(screen.getByText("A useful summary")).toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "Report heading" })).toBeInTheDocument()
    expect(screen.getByText("[Remote image omitted: remote]")).toBeInTheDocument()
  })

  it("clears the persistent Inspector selection with Escape", async () => {
    const user = userEvent.setup()
    const onClear = vi.fn()
    const entity: GraphEntityDetail = { id: "entity-1", short_id: null, title: "Alice", entity_type: "PERSON", degree: 1, rank: 1, description: null, community_ids: [], text_unit_ids: [] }
    render(<GraphInspector {...common} onClear={onClear} detail={{ kind: "entity", value: entity }} />)

    await user.click(screen.getByRole("region", { name: "Graph Inspector" }))
    await user.keyboard("{Escape}")
    expect(onClear).toHaveBeenCalledOnce()
  })
})
