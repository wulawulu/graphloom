import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { startQuery } from "@/api/client"
import { QueryComposer } from "@/components/query/query-composer"
import { setStudioLocale } from "@/i18n"

vi.mock("@/api/client", () => ({
  ApiError: class extends Error { status = 500 },
  startQuery: vi.fn(),
}))

beforeEach(() => {
  localStorage.clear()
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

async function openSettings(user: ReturnType<typeof userEvent.setup>): Promise<void> {
  await user.click(screen.getByRole("button", { name: "Query settings" }))
}

async function chooseSetting(user: ReturnType<typeof userEvent.setup>, setting: string, option: string): Promise<void> {
  await user.click(screen.getByRole("combobox", { name: setting }))
  await user.click(screen.getByRole("option", { name: option }))
}

async function submitQuestion(user: ReturnType<typeof userEvent.setup>, question = "Alice?"): Promise<void> {
  await user.type(screen.getByLabelText("Ask about the graph"), question)
  await user.click(screen.getByRole("button", { name: /Run .* Query/ }))
}

describe("QueryComposer", () => {
  it("submits the exact query on Enter through the form pipeline", async () => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    const input = screen.getByLabelText("Ask about the graph")
    await user.type(input, "王婆在其中起什么作用？")
    await user.keyboard("{Enter}")
    await waitFor(() => expect(startQuery).toHaveBeenCalledTimes(1))
    expect(startQuery).toHaveBeenCalledWith(expect.objectContaining({ query: "王婆在其中起什么作用？" }))
  })

  it("inserts a newline on Shift+Enter without submitting", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    const input = screen.getByLabelText("Ask about the graph")
    await user.type(input, "first")
    await user.keyboard("{Shift>}{Enter}{/Shift}")
    expect(input).toHaveValue("first\n")
    expect(startQuery).not.toHaveBeenCalled()
  })

  it("does not submit the Enter used to confirm an IME composition", async () => {
    vi.mocked(startQuery).mockResolvedValue(accepted)
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    const input = screen.getByLabelText("Ask about the graph")
    fireEvent.change(input, { target: { value: "王婆" } })
    fireEvent.compositionStart(input)
    fireEvent.keyDown(input, { key: "Enter", keyCode: 229, isComposing: true })
    expect(startQuery).not.toHaveBeenCalled()
    fireEvent.compositionEnd(input)
    fireEvent.keyDown(input, { key: "Enter", keyCode: 13 })
    await waitFor(() => expect(startQuery).toHaveBeenCalledTimes(1))
  })

  it("uses bounded auto-growing textarea styles and resets its height", async () => {
    const user = userEvent.setup()
    const { rerender } = render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    const input = screen.getByLabelText("Ask about the graph")
    expect(input).toHaveClass("min-h-14", "max-h-48", "overflow-y-hidden")
    expect(input).toHaveStyle({ height: "56px", overflowY: "hidden" })
    await user.type(input, "line one{shift>}{enter}{/shift}line two")
    rerender(<QueryComposer onAccepted={vi.fn()} resetRevision={1} />)
    expect(input).toHaveValue("")
    expect(input).toHaveStyle({ height: "56px", overflowY: "hidden" })
  })

  it("presents safe defaults while keeping settings collapsed", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    expect(screen.getByRole("combobox", { name: "Query method" })).toHaveTextContent("Local")
    expect(screen.queryByText("Standard")).not.toBeInTheDocument()
    expect(screen.queryByLabelText("Explainability detail")).not.toBeInTheDocument()
    expect(screen.queryByLabelText("Response style")).not.toBeInTheDocument()
    await openSettings(user)
    expect(screen.getByRole("switch", { name: "Developer Mode" })).toHaveAttribute("aria-checked", "false")
    expect(screen.getByLabelText("Response style")).toHaveTextContent("Standard")
    expect(screen.queryByLabelText("Explainability detail")).not.toBeInTheDocument()
  })

  it("submits content mode by default, then clears and refocuses the accepted query", async () => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    const onAccepted = vi.fn()
    render(<QueryComposer onAccepted={onAccepted} resetRevision={0} />)
    await submitQuestion(user)
    const input = screen.getByLabelText("Ask about the graph")
    expect(startQuery).toHaveBeenCalledWith({ query: "Alice?", method: "local", dynamic_community_selection: false, content_mode: "content", response_type: "Multiple Paragraphs" })
    expect(onAccepted).toHaveBeenCalledWith(accepted, "Alice?")
    await waitFor(() => expect(input).toHaveValue(""))
    expect(input).toHaveFocus()
    expect(input).toHaveStyle({ height: "56px", overflowY: "hidden" })
  })

  it("uses and persists Developer Mode only for debug Runs", async () => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    const view = render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await openSettings(user)
    await user.click(screen.getByRole("switch", { name: "Developer Mode" }))
    await submitQuestion(user)
    expect(startQuery).toHaveBeenCalledWith(expect.objectContaining({ content_mode: "debug" }))
    expect(localStorage.getItem("graphloom.studio.developerMode")).toBe("true")
    view.unmount()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await openSettings(user)
    expect(screen.getByRole("switch", { name: "Developer Mode" })).toHaveAttribute("aria-checked", "true")
  })

  it("preserves the query after an acceptance failure", async () => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockRejectedValue(new Error("unavailable"))
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await submitQuestion(user, "retry exactly")
    await waitFor(() => expect(screen.getByLabelText("Ask about the graph")).toHaveValue("retry exactly"))
  })

  it.each([["Concise", "Single Paragraph"], ["Detailed", "A detailed answer with sections and multiple paragraphs"]] as const)("maps %s response style to its stable prompt value", async (label, responseType) => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await openSettings(user)
    await chooseSetting(user, "Response style", label)
    await submitQuestion(user)
    expect(startQuery).toHaveBeenCalledWith(expect.objectContaining({ response_type: responseType }))
  })

  it("shows custom input only for Custom and preserves its exact payload", async () => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await openSettings(user)
    expect(screen.queryByLabelText("Custom response instructions")).not.toBeInTheDocument()
    await chooseSetting(user, "Response style", "Custom")
    const custom = screen.getByLabelText("Custom response instructions")
    expect(screen.getByRole("button", { name: "Run Local Query" })).toBeDisabled()
    await user.type(custom, "Answer in exactly 5 bullet points.")
    await submitQuestion(user)
    expect(startQuery).toHaveBeenCalledWith(expect.objectContaining({ response_type: "Answer in exactly 5 bullet points." }))
  })

  it("validates custom instructions using the backend UTF-8 byte limit", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await openSettings(user)
    await chooseSetting(user, "Response style", "Custom")
    await user.type(screen.getByLabelText("Ask about the graph"), "question")
    await user.type(screen.getByLabelText("Custom response instructions"), "界".repeat(86))
    expect(screen.getByText("Custom response instructions must be 256 bytes or fewer.")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Run Local Query" })).toBeDisabled()
  })

  it("does not leak a custom draft into a later preset request", async () => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await openSettings(user)
    await chooseSetting(user, "Response style", "Custom")
    await user.type(screen.getByLabelText("Custom response instructions"), "Five bullets")
    await chooseSetting(user, "Response style", "Detailed")
    expect(screen.queryByLabelText("Custom response instructions")).not.toBeInTheDocument()
    await submitQuestion(user)
    expect(startQuery).toHaveBeenCalledWith(expect.objectContaining({ response_type: "A detailed answer with sections and multiple paragraphs" }))
  })

  it("shows every user-visible Query mode in the primary selector", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await user.click(screen.getByRole("combobox", { name: "Query method" }))
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual(["Basic", "Local", "Global", "Dynamic Global", "DRIFT"])
  })

  it.each([["Basic", "basic", false], ["Global", "global", false], ["Dynamic Global", "global", true], ["DRIFT", "drift", false]] as const)("maps %s to its backend Query contract", async (label, method, dynamicCommunitySelection) => {
    const user = userEvent.setup()
    vi.mocked(startQuery).mockResolvedValue(accepted)
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await chooseQueryMethod(user, label)
    await submitQuestion(user, "same question")
    expect(startQuery).toHaveBeenCalledWith({ query: "same question", method, dynamic_community_selection: dynamicCommunitySelection, content_mode: "content", response_type: "Multiple Paragraphs" })
  })

  it("uses the current mode in the submitting state", async () => {
    const user = userEvent.setup()
    let resolveStart: ((value: typeof accepted) => void) | undefined
    vi.mocked(startQuery).mockReturnValue(new Promise((resolve) => { resolveStart = resolve }))
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await chooseQueryMethod(user, "DRIFT")
    await submitQuestion(user, "question")
    expect(screen.getByRole("button", { name: "Submitting DRIFT Query" })).toBeDisabled()
    expect(screen.getByRole("combobox", { name: "Query method" })).toBeDisabled()
    resolveStart?.(accepted)
    await waitFor(() => expect(screen.getByRole("button", { name: "Run DRIFT Query" })).toBeDisabled())
    expect(screen.getByLabelText("Ask about the graph")).toHaveValue("")
  })

  it("preserves all settings and the custom draft while switching Query modes", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await openSettings(user)
    await user.click(screen.getByRole("switch", { name: "Developer Mode" }))
    await chooseSetting(user, "Response style", "Custom")
    await user.type(screen.getByLabelText("Custom response instructions"), "Single Sentence")
    await chooseQueryMethod(user, "Dynamic Global")
    expect(screen.getByRole("switch", { name: "Developer Mode" })).toHaveAttribute("aria-checked", "true")
    expect(screen.getByLabelText("Response style")).toHaveTextContent("Custom")
    expect(screen.getByLabelText("Custom response instructions")).toHaveValue("Single Sentence")
  })

  it("clears only the query draft when reset revision advances", async () => {
    const user = userEvent.setup()
    const { rerender } = render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    const input = screen.getByLabelText("Ask about the graph")
    await user.type(input, "draft")
    await chooseQueryMethod(user, "Global")
    await openSettings(user)
    await user.click(screen.getByRole("switch", { name: "Developer Mode" }))
    await chooseSetting(user, "Response style", "Custom")
    await user.type(screen.getByLabelText("Custom response instructions"), "Five bullets")
    rerender(<QueryComposer onAccepted={vi.fn()} resetRevision={1} />)
    expect(input).toHaveValue("")
    expect(screen.getByRole("combobox", { name: "Query method" })).toHaveTextContent("Global")
    expect(screen.getByRole("switch", { name: "Developer Mode" })).toHaveAttribute("aria-checked", "true")
    expect(screen.getByLabelText("Response style")).toHaveTextContent("Custom")
    expect(screen.getByLabelText("Custom response instructions")).toHaveValue("Five bullets")
  })

  it("renders the Query settings presentation in Simplified Chinese", async () => {
    const user = userEvent.setup()
    await act(() => setStudioLocale("zh-CN", false))
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)

    expect(screen.getByRole("combobox", { name: "查询方式" })).toHaveTextContent("局部搜索")
    expect(screen.queryByText("标准")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "查询设置" }))
    expect(screen.getByRole("switch", { name: "开发者模式" })).toHaveAttribute("aria-checked", "false")
    expect(screen.getByLabelText("回答风格")).toHaveTextContent("标准")
    await user.click(screen.getByLabelText("回答风格"))
    expect(screen.getByRole("option", { name: "自定义" })).toBeInTheDocument()
  })

  it("keeps Query wire values identical in English and Simplified Chinese", async () => {
    const expectedRequest = {
      query: "same question",
      method: "global",
      dynamic_community_selection: true,
      content_mode: "content",
      response_type: "Multiple Paragraphs",
    }
    vi.mocked(startQuery).mockResolvedValue(accepted)

    const englishUser = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await chooseQueryMethod(englishUser, "Dynamic Global")
    await submitQuestion(englishUser, "same question")
    expect(startQuery).toHaveBeenLastCalledWith(expectedRequest)

    cleanup()
    await act(() => setStudioLocale("zh-CN", false))
    const chineseUser = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await chooseQueryMethodLocalized(chineseUser, "查询方式", "动态全局搜索")
    await submitQuestionLocalized(chineseUser, "same question")
    expect(startQuery).toHaveBeenLastCalledWith(expectedRequest)
  })

  it("changes locale without resetting Query business state or submitting a request", async () => {
    const user = userEvent.setup()
    render(<QueryComposer onAccepted={vi.fn()} resetRevision={0} />)
    await user.type(screen.getByLabelText("Ask about the graph"), "draft query")
    await chooseQueryMethod(user, "DRIFT")
    await openSettings(user)
    await user.click(screen.getByRole("switch", { name: "Developer Mode" }))
    await chooseSetting(user, "Response style", "Custom")
    await user.type(screen.getByLabelText("Custom response instructions"), "Answer in exactly 5 bullet points.")

    await act(() => setStudioLocale("zh-CN", false))

    expect(screen.getByLabelText("向图谱提问")).toHaveValue("draft query")
    expect(screen.getByRole("combobox", { name: "查询方式" })).toHaveTextContent("DRIFT 探索搜索")
    expect(screen.getByRole("switch", { name: "开发者模式" })).toHaveAttribute("aria-checked", "true")
    expect(screen.getByLabelText("回答风格")).toHaveTextContent("自定义")
    expect(screen.getByLabelText("自定义回答要求")).toHaveValue("Answer in exactly 5 bullet points.")
    expect(startQuery).not.toHaveBeenCalled()
  })
})

async function chooseQueryMethodLocalized(user: ReturnType<typeof userEvent.setup>, setting: string, label: string): Promise<void> {
  await user.click(screen.getByRole("combobox", { name: setting }))
  await user.click(screen.getByRole("option", { name: label }))
}

async function submitQuestionLocalized(user: ReturnType<typeof userEvent.setup>, question: string): Promise<void> {
  await user.type(screen.getByLabelText("向图谱提问"), question)
  await user.click(screen.getByRole("button", { name: "运行动态全局搜索" }))
}
