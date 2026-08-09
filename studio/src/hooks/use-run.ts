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

function isTerminal(run: ExplainabilityRun): boolean {
  return run.status === "completed" || run.status === "failed" || run.status === "cancelled"
}

export function useRun(runId: string | null): RunState & { refresh: () => void } {
  const [state, setState] = useState<RunState>(initialState)
  const [revision, setRevision] = useState(0)
  const refresh = useCallback(() => setRevision((value) => value + 1), [])

  useEffect(() => {
    if (runId === null) {
      setState(initialState)
      return undefined
    }
    setState({ ...initialState, loading: true })
    const controller = new AbortController()
    let timeout: ReturnType<typeof setTimeout> | undefined

    const load = async (): Promise<void> => {
      setState((current) => ({ ...current, loading: current.run === null, error: null }))
      try {
        const run = await getRun(runId, controller.signal)
        const result = await getQueryResult(runId, controller.signal)
        setState({ run, result, loading: false, error: null })
        if (!isTerminal(run)) {
          timeout = setTimeout(() => void load(), 2_000)
        }
      } catch (error) {
        if (controller.signal.aborted) return
        const message = error instanceof ApiError && error.status === 404
          ? "Run not found."
          : "Run metadata is unavailable."
        setState((current) => ({ ...current, loading: false, error: message }))
      }
    }
    void load()

    return () => {
      controller.abort()
      if (timeout !== undefined) clearTimeout(timeout)
    }
  }, [revision, runId])

  return { ...state, refresh }
}
