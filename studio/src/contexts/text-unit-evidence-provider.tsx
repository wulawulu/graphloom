import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { resolveTextUnits } from "@/api/client"
import type { GraphTextUnitRef } from "@/api/types"
import { TextUnitEvidenceContext, type TextUnitEvidenceContextValue, type TextUnitEvidenceStatus } from "@/contexts/text-unit-evidence"

const RESOLVE_BATCH_LIMIT = 100

interface EvidenceSnapshot {
  refs: ReadonlyMap<string, GraphTextUnitRef>
  statuses: ReadonlyMap<string, TextUnitEvidenceStatus>
}

export function TextUnitEvidenceProvider({ children }: { children: React.ReactNode }): React.ReactElement {
  const cache = useRef(new Map<string, GraphTextUnitRef>())
  const missing = useRef(new Set<string>())
  const failed = useRef(new Set<string>())
  const pending = useRef(new Set<string>())
  const requests = useRef(new Set<AbortController>())
  const requestQueue = useRef(Promise.resolve())
  const disposed = useRef(false)
  const [snapshot, setSnapshot] = useState<EvidenceSnapshot>({ refs: new Map(), statuses: new Map() })

  useEffect(() => () => {
    disposed.current = true
    requests.current.forEach((controller) => controller.abort())
    requests.current.clear()
  }, [])

  const publish = useCallback((): void => {
    const statuses = new Map<string, TextUnitEvidenceStatus>()
    cache.current.forEach((_reference, id) => statuses.set(id, "resolved"))
    pending.current.forEach((id) => statuses.set(id, "loading"))
    missing.current.forEach((id) => statuses.set(id, "unavailable"))
    failed.current.forEach((id) => statuses.set(id, "unavailable"))
    setSnapshot({ refs: new Map(cache.current), statuses })
  }, [])

  const resolve = useCallback((ids: readonly string[]): void => {
    const seen = new Set<string>()
    const unresolved = ids.filter((id) => {
      if (seen.has(id)) return false
      seen.add(id)
      return id.length > 0
        && !cache.current.has(id)
        && !missing.current.has(id)
        && !failed.current.has(id)
        && !pending.current.has(id)
    })
    if (unresolved.length === 0) return
    unresolved.forEach((id) => pending.current.add(id))
    publish()
    for (let offset = 0; offset < unresolved.length; offset += RESOLVE_BATCH_LIMIT) {
      const batch = unresolved.slice(offset, offset + RESOLVE_BATCH_LIMIT)
      requestQueue.current = requestQueue.current.then(async () => {
        if (disposed.current) return
        const controller = new AbortController()
        requests.current.add(controller)
        try {
          const response = await resolveTextUnits(batch, controller.signal)
          if (!controller.signal.aborted) {
            response.resolved.forEach((reference) => cache.current.set(reference.id, reference))
            response.missing_ids.forEach((id) => missing.current.add(id))
          }
        } catch (reason: unknown) {
          if (!(reason instanceof DOMException && reason.name === "AbortError")) batch.forEach((id) => failed.current.add(id))
        } finally {
          requests.current.delete(controller)
          batch.forEach((id) => pending.current.delete(id))
          if (!controller.signal.aborted) publish()
        }
      })
    }
  }, [publish])

  const value = useMemo<TextUnitEvidenceContextValue>(() => ({
    refs: snapshot.refs,
    resolve,
    status: (id) => snapshot.statuses.get(id) ?? "idle",
  }), [resolve, snapshot])

  return <TextUnitEvidenceContext.Provider value={value}>{children}</TextUnitEvidenceContext.Provider>
}
