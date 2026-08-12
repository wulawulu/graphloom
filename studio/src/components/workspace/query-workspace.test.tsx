import { useEffect } from "react"
import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { QueryWorkspace } from "@/components/workspace/query-workspace"

afterEach(cleanup)

describe("QueryWorkspace ownership", () => {
  it("keeps Answer and Run owners mounted while their sections collapse", async () => {
    const answerUnmounted = vi.fn()
    const runsUnmounted = vi.fn()
    function Owner({ children, onUnmount }: { children: React.ReactNode; onUnmount: () => void }): React.ReactElement {
      useEffect(() => onUnmount, [onUnmount])
      return <div>{children}</div>
    }
    const user = userEvent.setup()
    render(<QueryWorkspace composer={<div>Composer</div>} trace={<div>Trace</div>} answer={<Owner onUnmount={answerUnmounted}>Answer owner</Owner>} runs={<Owner onUnmount={runsUnmounted}>Run owner</Owner>} />)

    await user.click(screen.getByRole("button", { name: /Answer/ }))
    await user.click(screen.getByRole("button", { name: /Previous Runs/ }))
    await user.click(screen.getByRole("button", { name: /Previous Runs/ }))
    expect(screen.getByText("Answer owner")).toBeInTheDocument()
    expect(screen.getByText("Run owner")).toBeInTheDocument()
    expect(answerUnmounted).not.toHaveBeenCalled()
    expect(runsUnmounted).not.toHaveBeenCalled()
  })
})
