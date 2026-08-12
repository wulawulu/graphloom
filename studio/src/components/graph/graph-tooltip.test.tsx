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
    expect(clampTooltipPosition(630, 470, 640, 480, 240, 88)).toEqual({ x: 392, y: 384 })
    expect(clampTooltipPosition(20, 20, 180, 120, 164, 104)).toEqual({ x: 8, y: 8 })
  })

  it("remeasures when tooltip content changes at the same graph position", () => {
    const { rerender } = render(<GraphTooltip content={{ kind: "entity", value: { id: "a", title: "A", entity_type: null, rank: null } }} x={100} y={100} bounds={{ width: 300, height: 200 }} />)
    const tooltip = screen.getByRole("tooltip")
    Object.defineProperty(tooltip, "offsetWidth", { configurable: true, value: 260 })
    Object.defineProperty(tooltip, "offsetHeight", { configurable: true, value: 120 })

    rerender(<GraphTooltip content={{ kind: "relationship", value: { id: "r", source_entity_id: "a", target_entity_id: "b", source: "A long relationship source", target: "A long relationship target", weight: 1, rank: 2 } }} x={100} y={100} bounds={{ width: 300, height: 200 }} />)
    expect(tooltip).toHaveStyle({ left: "32px", top: "72px" })
  })
})
