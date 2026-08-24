import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { startQuery } from "@/api/client"
import { QueryComposer } from "@/components/query/query-composer"

vi.mock("@/api/client", () => ({
  ApiError: class extends Error { status = 500 },
  startQuery: vi.fn(),
}))

beforeEach(() => {
  HTMLElement.prototype.hasPointerCapture = () => false
  HTMLElement.prototype.setPointerCapture = () => undefined
  HTMLElement.prototype.releasePointerCapture = () => undefined
  HTMLElement.prototype.scrollIntoView = () => undefined
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const accepted = { run_id: "run-new", run_url: "", events_url: "", result_url: "" }

async function chooseQueryMethod(user: ReturnType<typeof userEvent.setup>, label: string): Promise<void> {
  await user.click(screen.getByRole("combobox", { name: "Query method" }))
  await user.click(screen.getByRole("option", { name: label }))
}

describe("QueryComposer", () => {
  it("keeps settings hidden by default and reveals content mode and response type on demand", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)

    expect(screen.getByRole("combobox", { name: "Query method" })).toHaveTextContent("Local")
    expect(screen.getByText("Metadata")).toBeInTheDocument()
    expect(screen.queryByLabelText("Explainability content")).not.toBeInTheDocument()
    expect(screen.queryByLabelText("Response type")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Query settings" }))
    expect(screen.getByLabelText("Explainability content")).toBeInTheDocument()
    expect(screen.getByLabelText("Response type")).toHaveValue("Multiple Paragraphs")
  })

  it("submits the Local defaults and reports the ephemeral submitted question", async () => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    const onAccepted = vi.fn()
    render(<QueryComposer onAccepted={onAccepted} resetRevision={0} />)

    await user.type(screen.getByLabelText("Ask about the graph"), "Alice?")
    await user.click(screen.getByRole("button", { name: "Run Local Query" }))
    expect(startQuery).toHaveBeenCalledWith({ query: "Alice?", method: "local", dynamic_community_selection: false, content_mode: "metadata", response_type: "Multiple Paragraphs" })
    expect(onAccepted).toHaveBeenCalledWith(accepted, "Alice?")
  })

  it("shows every user-visible Query mode in the primary selector", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)

    await user.click(screen.getByRole("combobox", { name: "Query method" }))
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual([
      "Basic",
      "Local",
      "Global",
      "Dynamic Global",
      "DRIFT",
    ])
  })

  it.each([
    ["Basic", "basic", false],
    ["Global", "global", false],
    ["Dynamic Global", "global", true],
    ["DRIFT", "drift", false],
  ] as const)("maps %s to its backend Query contract", async (label, method, dynamicCommunitySelection) => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)

    await chooseQueryMethod(user, label)
    expect(screen.getByRole("combobox", { name: "Query method" })).toHaveTextContent(label)
    expect(screen.getByRole("button", { name: `Run ${label} Query` })).toBeInTheDocument()
    await user.type(screen.getByLabelText("Ask about the graph"), "same question")
    await user.click(screen.getByRole("button", { name: `Run ${label} Query` }))

    expect(startQuery).toHaveBeenCalledWith({
      query: "same question",
      method,
      dynamic_community_selection: dynamicCommunitySelection,
      content_mode: "metadata",
      response_type: "Multiple Paragraphs",
    })
  })

  it("uses the current mode in the submitting state", async () => {
    const user = userEvent.setup()
    let resolveStart: ((value: typeof accepted) => void) | undefined
    vi.mocked(startQuery).mockReturnValue(new Promise((resolve) => { resolveStart = resolve }))
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)

    await chooseQueryMethod(user, "DRIFT")
    await user.type(screen.getByLabelText("Ask about the graph"), "question")
    await user.click(screen.getByRole("button", { name: "Run DRIFT Query" }))
    expect(screen.getByRole("button", { name: "Submitting DRIFT Query" })).toBeDisabled()
    expect(screen.getByRole("combobox", { name: "Query method" })).toBeDisabled()

    resolveStart?.(accepted)
    await waitFor(() => expect(screen.getByRole("button", { name: "Run DRIFT Query" })).toBeEnabled())
  })

  it("preserves explainability settings while switching Query modes", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)

    await user.click(screen.getByRole("button", { name: "Query settings" }))
    await user.click(screen.getByLabelText("Explainability content"))
    await user.click(screen.getByRole("option", { name: "Content" }))
    await user.clear(screen.getByLabelText("Response type"))
    await user.type(screen.getByLabelText("Response type"), "Single Sentence")
    await chooseQueryMethod(user, "Dynamic Global")

    expect(screen.getByLabelText("Explainability content")).toHaveTextContent("Content")
    expect(screen.getByLabelText("Response type")).toHaveValue("Single Sentence")
  })

  it("clears only the draft and preserves the Query mode when reset revision advances", async () => {
    const user = userEvent.setup()
    const { rerender } = render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    const input = screen.getByLabelText("Ask about the graph")
    await user.type(input, "draft")
    await chooseQueryMethod(user, "Global")

    rerender(<QueryComposer onAccepted={vi.fn()} resetRevision={1} />)
    expect(input).toHaveValue("")
    expect(screen.getByRole("combobox", { name: "Query method" })).toHaveTextContent("Global")
  })
})
