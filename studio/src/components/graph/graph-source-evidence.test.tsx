import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { getTextUnit } from "@/api/client"
import type { GraphTextUnitDetail, GraphTextUnitRef } from "@/api/types"
import { GraphSourceEvidence } from "@/components/graph/graph-source-evidence"
import { setStudioLocale } from "@/i18n"

vi.mock("@/api/client", () => ({ getTextUnit: vi.fn() }))

const sourceA: GraphTextUnitRef = { id: "71d89c81-1111-2222-3333-12345678e942", short_id: "184", preview: "西门庆因蔡京生辰而准备重礼。", n_tokens: 243 }
const sourceB: GraphTextUnitRef = { id: "text-2", short_id: "185", preview: "Second evidence", n_tokens: null }
const sources: GraphTextUnitRef[] = [sourceA, sourceB]

beforeEach(() => {
  vi.mocked(getTextUnit).mockReset()
  HTMLElement.prototype.hasPointerCapture = () => false
  HTMLElement.prototype.setPointerCapture = () => undefined
  HTMLElement.prototype.releasePointerCapture = () => undefined
})
afterEach(cleanup)

describe("GraphSourceEvidence", () => {
  it("renders readable evidence in artifact order without using UUIDs as primary labels", () => {
    render(<GraphSourceEvidence sourceIds={[sourceB.id, "missing-source-1234567890abcdef", sourceA.id, sourceB.id]} sources={sources} />)

    const buttons = screen.getAllByRole("button", { name: /View Text Unit/ })
    expect(buttons[0]).toHaveAccessibleName("View Text Unit 185")
    expect(buttons[1]).toHaveAccessibleName("View Text Unit 184")
    expect(screen.getByText("Second evidence")).toBeInTheDocument()
    expect(screen.getByText("243 Tokens")).toBeInTheDocument()
    expect(screen.getByText("Source unavailable")).toBeInTheDocument()
    expect(screen.getByText("missing-…cdef")).toBeInTheDocument()
    expect(screen.queryByText(sourceA.id)).not.toBeInTheDocument()
    expect(screen.getByRole("region", { name: "Source evidence · 3" })).toBeInTheDocument()
  })

  it("pluralizes English token counts", () => {
    render(<GraphSourceEvidence
      sourceIds={["one", "two"]}
      sources={[
        { id: "one", short_id: "1", preview: "One token", n_tokens: 1 },
        { id: "two", short_id: "2", preview: "Two tokens", n_tokens: 2 },
      ]}
    />)

    expect(screen.getByText("1 Token")).toBeInTheDocument()
    expect(screen.getByText("2 Tokens")).toBeInTheDocument()
  })

  it("localizes Studio chrome without translating source content", async () => {
    await setStudioLocale("zh-CN", false)
    render(<GraphSourceEvidence sourceIds={[sourceA.id]} sources={[sourceA]} />)

    expect(screen.getByRole("region", { name: "来源证据 · 1" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "查看 Text Unit 184" })).toBeInTheDocument()
    expect(screen.getByText("西门庆因蔡京生辰而准备重礼。")).toBeInTheDocument()
  })

  it("loads exact source text only after click and exposes bounded metadata", async () => {
    const detail: GraphTextUnitDetail = {
      id: sourceA.id,
      short_id: "184",
      text: "  西门庆原文\n第二行保持不变。  ",
      n_tokens: 243,
      document_id: "document-uuid",
    }
    vi.mocked(getTextUnit).mockResolvedValue(detail)
    const user = userEvent.setup()
    render(<GraphSourceEvidence sourceIds={[sourceA.id]} sources={[sourceA]} />)

    expect(getTextUnit).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "View Text Unit 184" }))

    expect(await screen.findByTestId("text-unit-exact-text")).toHaveTextContent("西门庆原文 第二行保持不变。")
    expect(screen.getByTestId("text-unit-exact-text").textContent).toBe(detail.text)
    expect(screen.getByText("document-uuid")).toBeInTheDocument()
    expect(screen.getByText(sourceA.id)).toHaveClass("break-all")
    expect(getTextUnit).toHaveBeenCalledWith(sourceA.id, expect.any(AbortSignal))
  })

  it("aborts closed requests and prevents stale source content from replacing a newer source", async () => {
    let resolveFirst: ((value: GraphTextUnitDetail) => void) | undefined
    vi.mocked(getTextUnit)
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve }))
      .mockResolvedValueOnce({ id: "text-2", short_id: "185", text: "Current B", n_tokens: null, document_id: null })
    const user = userEvent.setup()
    render(<GraphSourceEvidence sourceIds={[sourceA.id, sourceB.id]} sources={sources} />)

    await user.click(screen.getByRole("button", { name: "View Text Unit 184" }))
    const firstSignal = vi.mocked(getTextUnit).mock.calls[0]?.[1]
    expect(screen.getByText("Loading source…")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Close detail panel" }))
    expect(firstSignal?.aborted).toBe(true)
    await user.click(screen.getByRole("button", { name: "View Text Unit 185" }))
    expect(await screen.findByText("Current B")).toBeInTheDocument()

    await act(async () => {
      resolveFirst?.({ id: sourceA.id, short_id: "184", text: "STALE A", n_tokens: 243, document_id: null })
      await Promise.resolve()
    })
    expect(screen.queryByText("STALE A")).not.toBeInTheDocument()
    expect(screen.getByText("Current B")).toBeInTheDocument()
  })

  it("aborts a pending request when its owning detail unmounts and shows safe errors", async () => {
    vi.mocked(getTextUnit).mockReturnValueOnce(new Promise(() => undefined)).mockRejectedValueOnce(new Error("RAW_SOURCE_SECRET"))
    const user = userEvent.setup()
    const view = render(<GraphSourceEvidence sourceIds={[sourceA.id]} sources={[sourceA]} />)
    await user.click(screen.getByRole("button", { name: "View Text Unit 184" }))
    const signal = vi.mocked(getTextUnit).mock.calls[0]?.[1]
    view.unmount()
    expect(signal?.aborted).toBe(true)

    render(<GraphSourceEvidence sourceIds={[sourceB.id]} sources={[sourceB]} />)
    await user.click(screen.getByRole("button", { name: "View Text Unit 185" }))
    await waitFor(() => expect(screen.getByText("Source unavailable")).toBeInTheDocument())
    expect(screen.queryByText("RAW_SOURCE_SECRET")).not.toBeInTheDocument()
  })
})
