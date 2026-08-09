import { useCallback, useEffect, useState } from "react"

import { listRuns } from "@/api/client"
import type { ExplainabilityRun, RunHistoryCursor } from "@/api/types"

interface RunHistoryState {
  runs: ExplainabilityRun[]
  cursor: RunHistoryCursor | null
  loading: boolean
  error: string | null
}

export function useRunHistory(): RunHistoryState & { refresh: () => void; loadMore: () => void } {
  const [state, setState] = useState<RunHistoryState>({ runs: [], cursor: null, loading: true, error: null })
  const [revision, setRevision] = useState(0)
  const refresh = useCallback(() => setRevision((value) => value + 1), [])

  useEffect(() => {
    const controller = new AbortController()
    setState((current) => ({ ...current, loading: true, error: null }))
    void listRuns(undefined, controller.signal)
      .then((response) => setState({ runs: response.runs, cursor: response.next_cursor, loading: false, error: null }))
      .catch(() => {
        if (!controller.signal.aborted) {
          setState((current) => ({ ...current, loading: false, error: "Run history is unavailable." }))
        }
      })
    return () => controller.abort()
  }, [revision])

  const loadMore = useCallback(() => {
    if (state.cursor === null || state.loading) return
    const cursor = state.cursor
    setState((current) => ({ ...current, loading: true, error: null }))
    void listRuns(cursor)
      .then((response) => setState((current) => ({
        runs: [...current.runs, ...response.runs.filter((run) => !current.runs.some((existing) => existing.run_id === run.run_id))],
        cursor: response.next_cursor,
        loading: false,
        error: null,
      })))
      .catch(() => setState((current) => ({ ...current, loading: false, error: "Run history is unavailable." })))
  }, [state.cursor, state.loading])

  return { ...state, refresh, loadMore }
}
