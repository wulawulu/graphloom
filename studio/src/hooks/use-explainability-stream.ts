import { useEffect, useRef, useState } from "react"

import type { ExplainabilityEnvelope, RunStatus } from "@/api/types"
import { isTerminalEvent, mergeEnvelopes } from "@/lib/explainability"

export type StreamStatus = "idle" | "connecting" | "open" | "reconnecting" | "closed"

export interface EventSourceLike {
  readonly readyState: number
  close(): void
  addEventListener(type: string, listener: EventListener): void
  removeEventListener(type: string, listener: EventListener): void
  onopen: ((event: Event) => void) | null
  onerror: ((event: Event) => void) | null
}

export type EventSourceFactory = (url: string) => EventSourceLike

const browserEventSourceFactory: EventSourceFactory = (url) => new EventSource(url)

function isEnvelope(value: unknown): value is ExplainabilityEnvelope {
  if (typeof value !== "object" || value === null) return false
  const candidate = value as Partial<ExplainabilityEnvelope>
  return typeof candidate.sequence === "number"
    && candidate.sequence > 0
    && typeof candidate.record === "object"
    && candidate.record !== null
    && typeof candidate.record.event === "object"
    && candidate.record.event !== null
    && typeof candidate.record.event.type === "string"
}

export interface ExplainabilityConnectionOptions {
  url: string
  factory: EventSourceFactory
  onEnvelope: (envelope: ExplainabilityEnvelope) => void
  onStatus: (status: StreamStatus) => void
  onTerminal: () => void
}

export function createExplainabilityConnection(options: ExplainabilityConnectionOptions): () => void {
  const source = options.factory(options.url)
  let closed = false
  options.onStatus("connecting")

  const onEnvelope: EventListener = (event) => {
    if (!(event instanceof MessageEvent) || typeof event.data !== "string") return
    try {
      const parsed: unknown = JSON.parse(event.data)
      if (!isEnvelope(parsed)) return
      const eventSequence = Number.parseInt(event.lastEventId, 10)
      if (Number.isFinite(eventSequence) && eventSequence !== parsed.sequence) return
      options.onEnvelope(parsed)
      if (isTerminalEvent(parsed.record.event)) {
        closed = true
        source.close()
        options.onStatus("closed")
        options.onTerminal()
      }
    } catch {
      // A malformed frame is ignored; EventSource will continue with the next persisted envelope.
    }
  }
  source.addEventListener("explainability", onEnvelope)
  source.onopen = () => options.onStatus("open")
  source.onerror = () => {
    if (!closed) options.onStatus("reconnecting")
  }

  return () => {
    closed = true
    source.removeEventListener("explainability", onEnvelope)
    source.close()
    options.onStatus("closed")
  }
}

function isTerminalStatus(status: RunStatus | undefined): boolean {
  return status === "completed" || status === "failed" || status === "cancelled"
}

export function useExplainabilityStream(
  runId: string | null,
  runStatus: RunStatus | undefined,
  onTerminal: () => void,
  factory: EventSourceFactory = browserEventSourceFactory,
): { envelopes: ExplainabilityEnvelope[]; status: StreamStatus } {
  const [envelopes, setEnvelopes] = useState<ExplainabilityEnvelope[]>([])
  const [status, setStatus] = useState<StreamStatus>("idle")
  const closeRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    setEnvelopes([])
    if (runId === null) {
      setStatus("idle")
      return undefined
    }
    const close = createExplainabilityConnection({
      url: `/api/explainability/runs/${encodeURIComponent(runId)}/events`,
      factory,
      onEnvelope: (envelope) => setEnvelopes((current) => mergeEnvelopes(current, envelope)),
      onStatus: setStatus,
      onTerminal,
    })
    closeRef.current = close
    return () => {
      if (closeRef.current === close) closeRef.current = null
      close()
    }
  }, [factory, onTerminal, runId])

  useEffect(() => {
    if (isTerminalStatus(runStatus) && status === "reconnecting") {
      closeRef.current?.()
    }
  }, [runStatus, status])

  return { envelopes, status }
}
