import { useEffect, useRef, useState } from "react"

import type { QueryAnswerEvent } from "@/api/types"
import type { EventSourceFactory, EventSourceLike } from "@/hooks/use-explainability-stream"

export type QueryAnswerStreamStatus = "idle" | "connecting" | "streaming" | "completed" | "failed" | "reconnecting"

export interface QueryAnswerStreamState {
  text: string
  status: QueryAnswerStreamStatus
  hasStarted: boolean
  sequence: number
}

export type AnswerMergeResult = "applied" | "ignored" | "gap"

const initialState: QueryAnswerStreamState = {
  text: "",
  status: "idle",
  hasStarted: false,
  sequence: 0,
}

const browserEventSourceFactory: EventSourceFactory = (url) => new EventSource(url)

export function mergeQueryAnswerEvent(state: QueryAnswerStreamState, event: QueryAnswerEvent): { state: QueryAnswerStreamState; result: AnswerMergeResult } {
  if (event.type === "snapshot") {
    return {
      state: {
        text: event.text,
        sequence: event.sequence,
        hasStarted: event.text.length > 0,
        status: event.status === "streaming" ? "streaming" : event.status,
      },
      result: "applied",
    }
  }
  if (event.sequence <= state.sequence) return { state, result: "ignored" }
  if (event.sequence > state.sequence + 1) return { state, result: "gap" }
  if (event.type === "delta") {
    return {
      state: {
        text: state.text + event.delta,
        sequence: event.sequence,
        hasStarted: true,
        status: "streaming",
      },
      result: "applied",
    }
  }
  return {
    state: { ...state, sequence: event.sequence, status: event.type },
    result: "applied",
  }
}

function isQueryAnswerEvent(value: unknown): value is QueryAnswerEvent {
  if (typeof value !== "object" || value === null) return false
  const event = value as Partial<QueryAnswerEvent>
  if (typeof event.type !== "string" || typeof event.sequence !== "number" || !Number.isSafeInteger(event.sequence) || event.sequence < 0) return false
  if (event.type === "snapshot") return typeof event.text === "string" && (event.status === "streaming" || event.status === "completed" || event.status === "failed")
  if (event.type === "delta") return typeof event.delta === "string" && event.sequence > 0
  return (event.type === "completed" || event.type === "failed") && event.sequence > 0
}

interface ConnectionOptions {
  url: string
  factory: EventSourceFactory
  current: () => QueryAnswerStreamState
  onState: (state: QueryAnswerStreamState, terminal: boolean) => void
  onStatus: (status: QueryAnswerStreamStatus) => void
  onGap: () => void
  onTerminal: () => void
}

export function createQueryAnswerConnection(options: ConnectionOptions): () => void {
  const source: EventSourceLike = options.factory(options.url)
  let closed = false
  options.onStatus("connecting")
  const onAnswer: EventListener = (message) => {
    if (!(message instanceof MessageEvent) || typeof message.data !== "string") return
    try {
      const parsed: unknown = JSON.parse(message.data)
      if (!isQueryAnswerEvent(parsed)) return
      const frameSequence = Number.parseInt(message.lastEventId, 10)
      if (!Number.isFinite(frameSequence) || frameSequence !== parsed.sequence) return
      const merged = mergeQueryAnswerEvent(options.current(), parsed)
      if (merged.result === "gap") {
        closed = true
        source.close()
        options.onGap()
        return
      }
      if (merged.result === "ignored") return
      const terminal = merged.state.status === "completed" || merged.state.status === "failed"
      options.onState(merged.state, terminal)
      if (terminal) {
        closed = true
        source.close()
        options.onTerminal()
      }
    } catch {
      // Ignore malformed frames. The next authoritative snapshot reconciles reconnects.
    }
  }
  source.addEventListener("answer", onAnswer)
  source.onopen = () => options.onStatus("streaming")
  source.onerror = () => {
    if (!closed) options.onStatus("reconnecting")
  }
  return () => {
    closed = true
    source.removeEventListener("answer", onAnswer)
    source.close()
  }
}

export function useQueryAnswerStream(
  runId: string | null,
  onTerminal: () => void,
  factory: EventSourceFactory = browserEventSourceFactory,
): QueryAnswerStreamState {
  const [rendered, setRendered] = useState<{ runId: string | null; state: QueryAnswerStreamState }>({ runId: null, state: initialState })
  const [reconnectRevision, setReconnectRevision] = useState(0)
  const currentRef = useRef<QueryAnswerStreamState>(initialState)
  const frameRef = useRef<number | null>(null)
  const activeRunRef = useRef<string | null>(null)

  useEffect(() => {
    if (activeRunRef.current !== runId) {
      activeRunRef.current = runId
      currentRef.current = initialState
      setRendered({ runId, state: initialState })
    }
    if (runId === null) return undefined
    const flush = (): void => {
      frameRef.current = null
      setRendered({ runId, state: currentRef.current })
    }
    const scheduleFlush = (terminal: boolean): void => {
      if (terminal) {
        if (frameRef.current !== null) cancelAnimationFrame(frameRef.current)
        flush()
      } else if (frameRef.current === null) {
        frameRef.current = requestAnimationFrame(flush)
      }
    }
    const close = createQueryAnswerConnection({
      url: `/api/query/${encodeURIComponent(runId)}/events`,
      factory,
      current: () => currentRef.current,
      onState: (state, terminal) => {
        currentRef.current = state
        scheduleFlush(terminal)
      },
      onStatus: (status) => {
        currentRef.current = { ...currentRef.current, status }
        scheduleFlush(status === "completed" || status === "failed")
      },
      onGap: () => {
        currentRef.current = { ...currentRef.current, status: "reconnecting" }
        scheduleFlush(false)
        setReconnectRevision((value) => value + 1)
      },
      onTerminal,
    })
    return () => {
      close()
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    }
  }, [factory, onTerminal, reconnectRevision, runId])

  return rendered.runId === runId ? rendered.state : initialState
}
