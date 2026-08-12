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

function changeableMediaQuery(): MediaQueryList & { setMatches: (value: boolean) => void } {
  let listener: ((event: MediaQueryListEvent) => void) | undefined
  const query = {
    ...mobileMediaQuery(),
    matches: true,
    addEventListener: vi.fn((_type: string, callback: EventListenerOrEventListenerObject) => { if (typeof callback === "function") listener = callback as (event: MediaQueryListEvent) => void }),
    removeEventListener: vi.fn(),
    setMatches(value: boolean): void { query.matches = value; listener?.({ matches: value } as MediaQueryListEvent) },
  } as MediaQueryList & { matches: boolean; setMatches: (value: boolean) => void }
  return query
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

  it("keeps Query and Graph owners mounted across the desktop breakpoint", () => {
    const query = changeableMediaQuery()
    vi.stubGlobal("matchMedia", vi.fn(() => query))
    const onUnmount = vi.fn()
    function Owner(): React.ReactElement {
      useEffect(() => () => onUnmount(), [])
      return <div>Stable owner</div>
    }
    render(<StudioShell queryWorkspace={<div>Query</div>} graph={<Owner />} mobileTab="graph" onMobileTabChange={vi.fn()} />)

    query.setMatches(false)
    expect(screen.getByText("Stable owner")).toBeInTheDocument()
    expect(onUnmount).not.toHaveBeenCalled()
  })
})
