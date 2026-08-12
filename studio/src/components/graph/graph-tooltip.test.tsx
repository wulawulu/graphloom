import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"

import { GraphTooltip } from "@/components/graph/graph-tooltip"
import { clampTooltipPosition } from "@/lib/graph-tooltip"

afterEach(cleanup)

describe("GraphTooltip", () => {
  it("renders projection entity fields without requesting detail data", () => {
    render(<GraphTooltip content={{ kind: "entity", value: { id: "a", title: "Alice", entity_type: "PERSON", rank: 12 } }} x={20} y={20} bounds={{ width: 640, height: 480 }} />)
    expect(screen.getByRole("tooltip")).toHaveTextContent("Alice")
    expect(screen.getByText("PERSON · Rank 12")).toBeInTheDocument()
  })

  it("renders relationship direction and clamps within the graph canvas", () => {
    render(<GraphTooltip content={{ kind: "relationship", value: { id: "r", source_entity_id: "a", target_entity_id: "b", source: "Alice", target: "Bob", weight: 3.2, rank: 8 } }} x={630} y={470} bounds={{ width: 640, height: 480 }} />)
    expect(screen.getByText("Alice → Bob")).toBeInTheDocument()
    expect(screen.getByText("Weight 3.2 · Rank 8")).toBeInTheDocument()
    expect(clampTooltipPosition(630, 470, 640, 480)).toEqual({ x: 392, y: 384 })
  })
})
