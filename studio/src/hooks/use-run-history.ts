import { useCallback, useEffect, useRef, useState } from "react"

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
  const activeRequest = useRef<AbortController | null>(null)

  const beginRequest = useCallback((): AbortController => {
    activeRequest.current?.abort()
    const controller = new AbortController()
    activeRequest.current = controller
    return controller
  }, [])

  const refresh = useCallback(() => {
    const controller = beginRequest()
    setState((current) => ({ ...current, loading: true, error: null }))
    void listRuns(undefined, controller.signal)
      .then((response) => {
        if (activeRequest.current !== controller) return
        activeRequest.current = null
        setState({ runs: response.runs, cursor: response.next_cursor, loading: false, error: null })
      })
      .catch(() => {
        if (activeRequest.current !== controller || controller.signal.aborted) return
        activeRequest.current = null
        setState((current) => ({ ...current, loading: false, error: "Run history is unavailable." }))
      })
  }, [beginRequest])

  useEffect(() => {
    refresh()
    return () => {
      activeRequest.current?.abort()
      activeRequest.current = null
    }
  }, [refresh])

  const loadMore = useCallback(() => {
    if (state.cursor === null || state.loading) return
    const cursor = state.cursor
    const controller = beginRequest()
    setState((current) => ({ ...current, loading: true, error: null }))
    void listRuns(cursor, controller.signal)
      .then((response) => {
        if (activeRequest.current !== controller) return
        activeRequest.current = null
        setState((current) => ({
          runs: [...current.runs, ...response.runs.filter((run) => !current.runs.some((existing) => existing.run_id === run.run_id))],
          cursor: response.next_cursor,
          loading: false,
          error: null,
        }))
      })
      .catch(() => {
        if (activeRequest.current !== controller || controller.signal.aborted) return
        activeRequest.current = null
        setState((current) => ({ ...current, loading: false, error: "Run history is unavailable." }))
      })
  }, [beginRequest, state.cursor, state.loading])

  return { ...state, refresh, loadMore }
}
