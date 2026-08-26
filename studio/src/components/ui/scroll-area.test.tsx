import { createRef } from "react"
import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { ScrollArea } from "@/components/ui/scroll-area"

describe("ScrollArea", () => {
  it("exposes the Radix Viewport without changing the Root API", () => {
    const viewportRef = createRef<HTMLDivElement>()
    const { container } = render(
      <ScrollArea viewportRef={viewportRef} className="custom-root" aria-label="Results">
        <span>Latest result</span>
      </ScrollArea>,
    )

    expect(container.firstElementChild).toHaveClass("custom-root")
    expect(container.firstElementChild).toHaveAttribute("aria-label", "Results")
    expect(viewportRef.current).toContainElement(screen.getByText("Latest result"))
    expect(viewportRef.current).toHaveAttribute("data-radix-scroll-area-viewport")
  })
})
