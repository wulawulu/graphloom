import { renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { useRun } from "@/hooks/use-run"

afterEach(() => vi.unstubAllGlobals())

describe("selected Run lifecycle", () => {
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
})
