import { cleanup, render, screen } from "@testing-library/react"
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
    const props = { navigation: <div>Navigation owner</div>, timeline: <div>Timeline owner</div>, graph: <div>Graph owner</div>, answer: <div>Answer owner</div>, onMobileTabChange: vi.fn() }
    const { rerender } = render(<StudioShell {...props} mobileTab="runs" />)
    expect(screen.queryByText("Graph owner")).not.toBeInTheDocument()

    rerender(<StudioShell {...props} mobileTab="graph" />)
    expect(screen.getAllByText("Graph owner")).toHaveLength(1)
  })
})
