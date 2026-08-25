import { act, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { useRun } from "@/hooks/use-run"

afterEach(() => vi.unstubAllGlobals())

describe("selected Run lifecycle", () => {
  it("loads an active Run once without fixed-interval result polling", async () => {
    vi.useFakeTimers()
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (url.endsWith("/result")) return Promise.resolve(new Response("waiting", { status: 202 }))
      return Promise.resolve(new Response(JSON.stringify({ run_id: "running", status: "running" }), { status: 200 }))
    })
    vi.stubGlobal("fetch", fetchMock)
    renderHook(() => useRun("running"))
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2))
    await vi.advanceTimersByTimeAsync(10_000)
    expect(fetchMock).toHaveBeenCalledTimes(2)
    vi.useRealTimers()
  })

  it("clears terminal metadata and result immediately when switching Runs", async () => {
    let resolveRunning: ((response: Response) => void) | undefined
    const runningResponse = new Promise<Response>((resolve) => { resolveRunning = resolve })
    vi.stubGlobal("fetch", vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (url.endsWith("/completed")) {
        return Promise.resolve(new Response(JSON.stringify({ run_id: "completed", status: "completed" }), { status: 200 }))
      }
      if (url.endsWith("/completed/result")) {
        return Promise.resolve(new Response(JSON.stringify({ run_id: "completed", response: "old answer", elapsed_ms: 1, usage: { llm_calls: 1, prompt_tokens: 1, output_tokens: 1, categories: {} } }), { status: 200 }))
      }
      if (url.endsWith("/running")) return runningResponse
      return Promise.resolve(new Response("missing", { status: 404 }))
    }))

    const { result, rerender } = renderHook(
      ({ runId }) => useRun(runId),
      { initialProps: { runId: "completed" } },
    )
    await waitFor(() => expect(result.current.result.state).toBe("ready"))

    rerender({ runId: "running" })
    await waitFor(() => expect(result.current.run).toBeNull())
    expect(result.current.result.state).toBe("waiting")
    expect(result.current.loading).toBe(true)

    resolveRunning?.(new Response(JSON.stringify({ run_id: "running", status: "running" }), { status: 200 }))
  })

  it("retries a terminal-driven refresh after one transient result failure", async () => {
    vi.useFakeTimers()
    let resultRequests = 0
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (!url.endsWith("/result")) {
        return Promise.resolve(new Response(JSON.stringify({ run_id: "run", status: resultRequests === 0 ? "running" : "completed" }), { status: 200 }))
      }
      resultRequests += 1
      if (resultRequests === 1) return Promise.resolve(new Response("waiting", { status: 202 }))
      if (resultRequests === 2) return Promise.resolve(new Response("temporary", { status: 503 }))
      return Promise.resolve(new Response(JSON.stringify({ run_id: "run", response: "canonical", elapsed_ms: 1, usage: { llm_calls: 1, prompt_tokens: 1, output_tokens: 1, categories: {} } }), { status: 200 }))
    })
    vi.stubGlobal("fetch", fetchMock)
    const { result } = renderHook(() => useRun("run"))
    await vi.waitFor(() => expect(result.current.result.state).toBe("waiting"))

    act(() => result.current.refreshTerminal())
    await vi.waitFor(() => expect(resultRequests).toBe(2))
    await act(() => vi.advanceTimersByTimeAsync(100))
    await vi.waitFor(() => expect(result.current.result.state).toBe("ready"))
    expect(fetchMock).toHaveBeenCalledTimes(6)
    vi.useRealTimers()
  })

  it("does not carry terminal retries into a newly selected running Run", async () => {
    vi.useFakeTimers()
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (url.endsWith("/result")) return Promise.resolve(new Response("waiting", { status: 202 }))
      const runId = url.endsWith("run-b") ? "run-b" : "run-a"
      return Promise.resolve(new Response(JSON.stringify({ run_id: runId, status: "running" }), { status: 200 }))
    })
    vi.stubGlobal("fetch", fetchMock)
    const { result, rerender } = renderHook(
      ({ runId }) => useRun(runId),
      { initialProps: { runId: "run-a" } },
    )
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2))
    act(() => result.current.refreshTerminal())
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(4))

    rerender({ runId: "run-b" })
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(6))
    await act(() => vi.advanceTimersByTimeAsync(1_000))
    expect(fetchMock).toHaveBeenCalledTimes(6)
    vi.useRealTimers()
  })
})
