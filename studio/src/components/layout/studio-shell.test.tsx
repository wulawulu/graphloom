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
    media: "(min-width: 1280px)",
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }
}

describe("StudioShell responsive ownership", () => {
  it("keeps one Graph Explorer mounted while switching mobile tabs", () => {
    vi.stubGlobal("matchMedia", vi.fn(() => mobileMediaQuery()))
    const props = { queryWorkspace: <div>Query owner</div>, graph: <div>Graph owner</div>, onMobileTabChange: vi.fn() }
    const { rerender } = render(<StudioShell {...props} mobileTab="query" />)
    expect(screen.getByText("Graph owner")).toBeInTheDocument()

    rerender(<StudioShell {...props} mobileTab="graph" />)
    expect(screen.getAllByText("Graph owner")).toHaveLength(1)
  })

  it("keeps the primary graph mounted when the query workspace collapses", async () => {
    vi.stubGlobal("matchMedia", vi.fn(() => ({ ...mobileMediaQuery(), matches: true })))
    const onUnmount = vi.fn()
    function PersistentOwner({ children }: { children: React.ReactNode }): React.ReactElement {
      useEffect(() => () => onUnmount(), [])
      return <div>{children}</div>
    }
    const user = userEvent.setup()
    render(<StudioShell queryWorkspace={<PersistentOwner>Query owner</PersistentOwner>} graph={<PersistentOwner>Persistent graph</PersistentOwner>} mobileTab="query" onMobileTabChange={vi.fn()} />)

    await user.click(screen.getByRole("button", { name: "Collapse query workspace" }))
    expect(screen.getByText("Persistent graph")).toBeInTheDocument()
    expect(onUnmount).not.toHaveBeenCalled()
  })
})
