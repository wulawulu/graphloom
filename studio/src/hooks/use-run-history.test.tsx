import { act, renderHook, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { listRuns } from "@/api/client"
import type { ExplainabilityRun, RunHistoryResponse } from "@/api/types"
import { useRunHistory } from "@/hooks/use-run-history"

vi.mock("@/api/client", () => ({ listRuns: vi.fn() }))

function run(runId: string): ExplainabilityRun {
  return {
    run_id: runId,
    kind: "query",
    status: "completed",
    started_at: "2026-08-09T00:00:00.000000000Z",
    event_count: 1,
  }
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolvePromise: ((value: T) => void) | undefined
  const promise = new Promise<T>((resolve) => { resolvePromise = resolve })
  return {
    promise,
    resolve: (value) => resolvePromise?.(value),
  }
}

beforeEach(() => vi.mocked(listRuns).mockReset())

describe("Run History request ownership", () => {
  it("prevents a stale loadMore response from appending after refresh", async () => {
    const loadMore = deferred<RunHistoryResponse>()
    const refresh = deferred<RunHistoryResponse>()
    vi.mocked(listRuns)
      .mockResolvedValueOnce({ runs: [run("old-first")], next_cursor: { started_at: "2026-08-08T00:00:00.000000000Z", run_id: "old-first" } })
      .mockReturnValueOnce(loadMore.promise)
      .mockReturnValueOnce(refresh.promise)
    const { result } = renderHook(() => useRunHistory())
    await waitFor(() => expect(result.current.runs.map((value) => value.run_id)).toEqual(["old-first"]))

    act(() => result.current.loadMore())
    await waitFor(() => expect(listRuns).toHaveBeenCalledTimes(2))
    const loadMoreSignal = vi.mocked(listRuns).mock.calls[1]?.[1]
    act(() => result.current.refresh())
    await waitFor(() => expect(listRuns).toHaveBeenCalledTimes(3))
    expect(loadMoreSignal?.aborted).toBe(true)

    await act(async () => refresh.resolve({ runs: [run("fresh-first")], next_cursor: null }))
    await waitFor(() => expect(result.current.runs.map((value) => value.run_id)).toEqual(["fresh-first"]))
    await act(async () => loadMore.resolve({ runs: [run("stale-more")], next_cursor: { started_at: "2026-08-07T00:00:00.000000000Z", run_id: "stale-more" } }))
    expect(result.current.runs.map((value) => value.run_id)).toEqual(["fresh-first"])
    expect(result.current.cursor).toBeNull()
    expect(result.current.loading).toBe(false)
  })

  it("keeps the newest refresh when an older refresh resolves late", async () => {
    const firstRefresh = deferred<RunHistoryResponse>()
    const secondRefresh = deferred<RunHistoryResponse>()
    vi.mocked(listRuns)
      .mockResolvedValueOnce({ runs: [run("initial")], next_cursor: null })
      .mockReturnValueOnce(firstRefresh.promise)
      .mockReturnValueOnce(secondRefresh.promise)
    const { result } = renderHook(() => useRunHistory())
    await waitFor(() => expect(result.current.runs.map((value) => value.run_id)).toEqual(["initial"]))

    act(() => result.current.refresh())
    await waitFor(() => expect(listRuns).toHaveBeenCalledTimes(2))
    const firstSignal = vi.mocked(listRuns).mock.calls[1]?.[1]
    act(() => result.current.refresh())
    await waitFor(() => expect(listRuns).toHaveBeenCalledTimes(3))
    expect(firstSignal?.aborted).toBe(true)

    await act(async () => secondRefresh.resolve({ runs: [run("newest")], next_cursor: null }))
    await waitFor(() => expect(result.current.runs.map((value) => value.run_id)).toEqual(["newest"]))
    await act(async () => firstRefresh.resolve({ runs: [run("stale")], next_cursor: null }))
    expect(result.current.runs.map((value) => value.run_id)).toEqual(["newest"])
    expect(result.current.error).toBeNull()
    expect(result.current.loading).toBe(false)
  })
})
