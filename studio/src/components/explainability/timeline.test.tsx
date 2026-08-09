import { render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { Timeline } from "@/components/explainability/timeline"

describe("Timeline", () => {
  it("renders the empty state and a forward-compatible unknown event", () => {
    const { rerender } = render(<Timeline runId={null} envelopes={[]} streamStatus="idle" onFocusGraph={vi.fn()} />)
    expect(screen.getByText("No Run selected")).toBeInTheDocument()
    rerender(<Timeline runId="run" streamStatus="open" onFocusGraph={vi.fn()} envelopes={[{ schema_version: 1, sequence: 4, record: { run_id: "run", timestamp: "2026-08-09T00:00:00Z", span_id: "span", event: { type: "future_graphloom_event", foo: "bar" } } }]} />)
    expect(screen.getByText("future graphloom event")).toBeInTheDocument()
    expect(screen.getByText("#4")).toBeInTheDocument()
  })
})
