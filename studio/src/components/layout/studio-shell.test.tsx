import { useEffect } from "react"
import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { StudioShell } from "@/components/layout/studio-shell"

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

function mobileMediaQuery(): MediaQueryList {
  return {
    matches: false,
    media: "(min-width: 1024px)",
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }
}

describe("StudioShell responsive ownership", () => {
  it("does not mount a hidden Graph Explorer on mobile and mounts one when selected", () => {
    vi.stubGlobal("matchMedia", vi.fn(() => mobileMediaQuery()))
    const props = { queryWorkspace: <div>Query owner</div>, graph: <div>Graph owner</div>, onMobileTabChange: vi.fn() }
    const { rerender } = render(<StudioShell {...props} mobileTab="query" />)
    expect(screen.queryByText("Graph owner")).not.toBeInTheDocument()

    rerender(<StudioShell {...props} mobileTab="graph" />)
    expect(screen.getAllByText("Graph owner")).toHaveLength(1)
  })

  it("keeps the primary graph mounted when the query workspace collapses", async () => {
    vi.stubGlobal("matchMedia", vi.fn(() => ({ ...mobileMediaQuery(), matches: true })))
    const onUnmount = vi.fn()
    function GraphOwner(): React.ReactElement {
      useEffect(() => () => onUnmount(), [])
      return <div>Persistent graph</div>
    }
    const user = userEvent.setup()
    render(<StudioShell queryWorkspace={<div>Query owner</div>} graph={<GraphOwner />} mobileTab="query" onMobileTabChange={vi.fn()} />)

    await user.click(screen.getByRole("button", { name: "Collapse query workspace" }))
    expect(screen.getByText("Persistent graph")).toBeInTheDocument()
    expect(onUnmount).not.toHaveBeenCalled()
  })
})
