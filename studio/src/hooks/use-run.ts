import { useCallback, useEffect, useState } from "react"

import { ApiError, getQueryResult, getRun } from "@/api/client"
import type { ExplainabilityRun, QueryResultState } from "@/api/types"

interface RunState {
  run: ExplainabilityRun | null
  result: QueryResultState
  loading: boolean
  error: string | null
}

const initialState: RunState = {
  run: null,
  result: { state: "waiting" },
  loading: false,
  error: null,
}

interface RefreshRequest {
  revision: number
  terminal: boolean
  runId: string | null
}

const TERMINAL_RETRY_DELAYS_MS = [100, 300] as const

export function useRun(runId: string | null): RunState & { refresh: () => void; refreshTerminal: () => void } {
  const [state, setState] = useState<RunState>(initialState)
  const [request, setRequest] = useState<RefreshRequest>({ revision: 0, terminal: false, runId: null })
  const refresh = useCallback(() => setRequest((value) => ({ revision: value.revision + 1, terminal: false, runId })), [runId])
  const refreshTerminal = useCallback(() => setRequest((value) => ({ revision: value.revision + 1, terminal: true, runId })), [runId])

  useEffect(() => {
    if (runId === null) {
      setState(initialState)
      return undefined
    }
    setState({ ...initialState, loading: true })
    const controller = new AbortController()
    let retryTimer: number | undefined
    const isTerminalRefresh = request.terminal && request.runId === runId
    const load = async (retryIndex = 0): Promise<void> => {
      setState((current) => ({ ...current, loading: current.run === null, error: null }))
      try {
        const run = await getRun(runId, controller.signal)
        const result = await getQueryResult(runId, controller.signal)
        setState({ run, result, loading: false, error: null })
        if (isTerminalRefresh && result.state === "waiting" && retryIndex < TERMINAL_RETRY_DELAYS_MS.length) {
          retryTimer = window.setTimeout(() => void load(retryIndex + 1), TERMINAL_RETRY_DELAYS_MS[retryIndex])
        }
      } catch (error) {
        if (controller.signal.aborted) return
        if (isTerminalRefresh && !(error instanceof ApiError && error.status === 404) && retryIndex < TERMINAL_RETRY_DELAYS_MS.length) {
          retryTimer = window.setTimeout(() => void load(retryIndex + 1), TERMINAL_RETRY_DELAYS_MS[retryIndex])
          return
        }
        const message = error instanceof ApiError && error.status === 404
          ? "Run not found."
          : "Run metadata is unavailable."
        setState((current) => ({ ...current, loading: false, error: message }))
      }
    }
    void load()

    return () => {
      controller.abort()
      if (retryTimer !== undefined) window.clearTimeout(retryTimer)
    }
  }, [request, runId])

  return { ...state, refresh, refreshTerminal }
}
