import { act, renderHook } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { QueryAnswerEvent } from "@/api/types"
import type { EventSourceFactory, EventSourceLike } from "@/hooks/use-explainability-stream"
import {
  createQueryAnswerConnection,
  mergeQueryAnswerEvent,
  type QueryAnswerStreamState,
  useQueryAnswerStream,
} from "@/hooks/use-query-answer-stream"

class FakeEventSource implements EventSourceLike {
  readyState = 0
  onopen: ((event: Event) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  closed = false
  private readonly listeners = new Map<string, Set<EventListener>>()

  close(): void { this.closed = true }
  addEventListener(type: string, listener: EventListener): void {
    const values = this.listeners.get(type) ?? new Set<EventListener>()
    values.add(listener)
    this.listeners.set(type, values)
  }
  removeEventListener(type: string, listener: EventListener): void { this.listeners.get(type)?.delete(listener) }
  emit(event: QueryAnswerEvent): void {
    const message = new MessageEvent("answer", { data: JSON.stringify(event), lastEventId: String(event.sequence) })
    for (const listener of this.listeners.get("answer") ?? []) listener(message)
  }
}

const empty: QueryAnswerStreamState = { text: "", status: "connecting", hasStarted: false, sequence: 0 }

describe("Query answer stream", () => {
  it("authoritatively replaces snapshots, appends contiguous deltas, and ignores duplicates", () => {
    const snapshot = mergeQueryAnswerEvent(empty, { type: "snapshot", sequence: 2, text: "AB", status: "streaming" })
    expect(snapshot.state).toEqual({ text: "AB", status: "streaming", hasStarted: true, sequence: 2 })
    const delta = mergeQueryAnswerEvent(snapshot.state, { type: "delta", sequence: 3, delta: "C" })
    expect(delta.state.text).toBe("ABC")
    expect(mergeQueryAnswerEvent(delta.state, { type: "delta", sequence: 3, delta: "C" }).result).toBe("ignored")
    expect(mergeQueryAnswerEvent(delta.state, { type: "delta", sequence: 5, delta: "E" }).result).toBe("gap")
  })

  it("reconciles reconnect snapshots without duplicate text and preserves partial failure", () => {
    let state = mergeQueryAnswerEvent(empty, { type: "snapshot", sequence: 0, text: "", status: "streaming" }).state
    state = mergeQueryAnswerEvent(state, { type: "delta", sequence: 1, delta: "A" }).state
    state = mergeQueryAnswerEvent(state, { type: "delta", sequence: 2, delta: "B" }).state
    state = mergeQueryAnswerEvent(state, { type: "snapshot", sequence: 4, text: "ABCD", status: "streaming" }).state
    state = mergeQueryAnswerEvent(state, { type: "delta", sequence: 5, delta: "E" }).state
    state = mergeQueryAnswerEvent(state, { type: "failed", sequence: 6 }).state
    expect(state).toEqual({ text: "ABCDE", status: "failed", hasStarted: true, sequence: 6 })
  })

  it("closes and requests a fresh snapshot on a sequence gap", () => {
    const source = new FakeEventSource()
    let state = empty
    const gap = vi.fn()
    createQueryAnswerConnection({
      url: "/events",
      factory: () => source,
      current: () => state,
      onState: (next) => { state = next },
      onStatus: vi.fn(),
      onGap: gap,
      onTerminal: vi.fn(),
    })
    source.emit({ type: "snapshot", sequence: 1, text: "A", status: "streaming" })
    source.emit({ type: "delta", sequence: 3, delta: "C" })
    expect(gap).toHaveBeenCalledOnce()
    expect(source.closed).toBe(true)
    expect(state.text).toBe("A")
  })

  it("closes the old EventSource and clears its buffer when switching Runs", () => {
    const sources: FakeEventSource[] = []
    const factory: EventSourceFactory = () => {
      const source = new FakeEventSource()
      sources.push(source)
      return source
    }
    const terminal = vi.fn()
    const { result, rerender } = renderHook(
      ({ runId }) => useQueryAnswerStream(runId, terminal, factory),
      { initialProps: { runId: "run-a" as string | null } },
    )
    act(() => {
      sources[0]?.emit({ type: "snapshot", sequence: 1, text: "A", status: "streaming" })
    })
    rerender({ runId: "run-b" })
    expect(sources[0]?.closed).toBe(true)
    expect(result.current.text).toBe("")
    act(() => {
      sources[0]?.emit({ type: "delta", sequence: 2, delta: "late" })
      sources[1]?.emit({ type: "snapshot", sequence: 0, text: "", status: "streaming" })
    })
    expect(result.current.text).toBe("")
  })
})
