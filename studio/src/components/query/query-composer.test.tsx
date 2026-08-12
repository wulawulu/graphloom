import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { startQuery } from "@/api/client"
import { QueryComposer } from "@/components/query/query-composer"

vi.mock("@/api/client", () => ({
  ApiError: class extends Error { status = 500 },
  startQuery: vi.fn(),
}))

afterEach(cleanup)

describe("QueryComposer", () => {
  it("keeps settings hidden by default and reveals content mode and response type on demand", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)

    expect(screen.getByText("Local")).toBeInTheDocument()
    expect(screen.getByText("Metadata")).toBeInTheDocument()
    expect(screen.queryByLabelText("Explainability content")).not.toBeInTheDocument()
    expect(screen.queryByLabelText("Response type")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Query settings" }))
    expect(screen.getByLabelText("Explainability content")).toBeInTheDocument()
    expect(screen.getByLabelText("Response type")).toHaveValue("Multiple Paragraphs")
  })

  it("submits the Local defaults and reports the ephemeral submitted question", async () => {
    const user = userEvent.setup()
    const accepted = { run_id: "run-new", run_url: "", events_url: "", result_url: "" }
    vi.mocked(startQuery).mockResolvedValue(accepted)
    const onAccepted = vi.fn()
    render(<QueryComposer onAccepted={onAccepted} resetRevision={0} />)

    await user.type(screen.getByLabelText("Ask about the graph"), "Alice?")
    await user.click(screen.getByRole("button", { name: "Run Local Query" }))
    expect(startQuery).toHaveBeenCalledWith({ query: "Alice?", method: "local", content_mode: "metadata", response_type: "Multiple Paragraphs" })
    expect(onAccepted).toHaveBeenCalledWith(accepted, "Alice?")
  })

  it("clears only the draft when New Query advances the reset revision", async () => {
    const user = userEvent.setup()
    const { rerender } = render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    const input = screen.getByLabelText("Ask about the graph")
    await user.type(input, "draft")

    rerender(<QueryComposer onAccepted={vi.fn()} resetRevision={1} />)
    expect(input).toHaveValue("")
  })
})
