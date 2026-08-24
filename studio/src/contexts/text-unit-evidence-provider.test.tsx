import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import { useEffect } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { resolveTextUnits } from "@/api/client"
import type { GraphTextUnitResolveResponse } from "@/api/types"
import { useTextUnitEvidence } from "@/contexts/text-unit-evidence"
import { TextUnitEvidenceProvider } from "@/contexts/text-unit-evidence-provider"

vi.mock("@/api/client", () => ({ resolveTextUnits: vi.fn() }))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function EvidenceConsumer({ id }: { id: string }): React.ReactElement {
  const { refs, resolve, status } = useTextUnitEvidence()
  useEffect(() => resolve([id]), [resolve, id])
  return <span>{refs.get(id)?.preview ?? status(id)}</span>
}

describe("TextUnitEvidenceProvider", () => {
  it("deduplicates concurrent consumers through one Run-scoped batch", async () => {
    vi.mocked(resolveTextUnits).mockResolvedValue({ resolved: [{ id: "text-a", short_id: "184", preview: "shared preview", n_tokens: 12 }], missing_ids: [] })
    render(<TextUnitEvidenceProvider><EvidenceConsumer id="text-a" /><EvidenceConsumer id="text-a" /></TextUnitEvidenceProvider>)

    await waitFor(() => expect(screen.getAllByText("shared preview")).toHaveLength(2))
    expect(resolveTextUnits).toHaveBeenCalledTimes(1)
  })

  it("aborts the previous Run and ignores its late response", async () => {
    let resolveRunA: ((value: GraphTextUnitResolveResponse) => void) | undefined
    let runASignal: AbortSignal | undefined
    vi.mocked(resolveTextUnits).mockImplementation((ids, signal) => {
      if (ids[0] === "run-a") {
        runASignal = signal
        return new Promise((resolve) => { resolveRunA = resolve })
      }
      return Promise.resolve({ resolved: [{ id: "run-b", short_id: "2", preview: "Run B preview", n_tokens: null }], missing_ids: [] })
    })
    const view = render(<TextUnitEvidenceProvider key="run-a"><EvidenceConsumer id="run-a" /></TextUnitEvidenceProvider>)
    await waitFor(() => expect(resolveTextUnits).toHaveBeenCalledTimes(1))

    view.rerender(<TextUnitEvidenceProvider key="run-b"><EvidenceConsumer id="run-b" /></TextUnitEvidenceProvider>)
    await waitFor(() => expect(screen.getByText("Run B preview")).toBeInTheDocument())
    expect(runASignal?.aborted).toBe(true)
    await act(async () => resolveRunA?.({ resolved: [{ id: "run-a", short_id: "1", preview: "late Run A preview", n_tokens: null }], missing_ids: [] }))
    expect(screen.queryByText("late Run A preview")).not.toBeInTheDocument()
  })
})
