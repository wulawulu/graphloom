import { act, renderHook } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { ExplainabilityEnvelope } from "@/api/types"
import {
  createExplainabilityConnection,
  type EventSourceFactory,
  type EventSourceLike,
  useExplainabilityStream,
} from "@/hooks/use-explainability-stream"

class FakeEventSource implements EventSourceLike {
  readyState = 0
  onopen: ((event: Event) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  closed = false
  private readonly listeners = new Map<string, Set<EventListener>>()

  close(): void { this.closed = true }
  addEventListener(type: string, listener: EventListener): void {
    const values = this.listeners.get(type) ?? new Set<EventListener>()
    values.add(listener); this.listeners.set(type, values)
  }
  removeEventListener(type: string, listener: EventListener): void { this.listeners.get(type)?.delete(listener) }
  emit(envelope: ExplainabilityEnvelope): void {
    const event = new MessageEvent("explainability", { data: JSON.stringify(envelope), lastEventId: String(envelope.sequence) })
    for (const listener of this.listeners.get("explainability") ?? []) listener(event)
  }
}

function envelope(sequence: number, type: string): ExplainabilityEnvelope {
  return { schema_version: 1, sequence, record: { run_id: "run-1", timestamp: "2026-08-09T00:00:00.000000000Z", span_id: "span", event: { type } } }
}

describe("Explainability EventSource lifecycle", () => {
  it("opens, reports errors, receives frames, and closes on terminal events", () => {
    const source = new FakeEventSource()
    const statuses: string[] = []
    const received: number[] = []
    const terminal = vi.fn()
    const close = createExplainabilityConnection({
      url: "/events",
      factory: () => source,
      onEnvelope: (value) => received.push(value.sequence),
      onStatus: (value) => statuses.push(value),
      onTerminal: terminal,
    })
    source.onopen?.(new Event("open"))
    source.emit(envelope(2, "future_graphloom_event"))
    source.onerror?.(new Event("error"))
    source.emit(envelope(3, "run_completed"))
    expect(received).toEqual([2, 3])
    expect(statuses).toEqual(["connecting", "open", "reconnecting", "closed"])
    expect(source.closed).toBe(true)
    expect(terminal).toHaveBeenCalledOnce()
    close()
  })

  it("deduplicates by sequence, sorts frames, and closes the old Run on switch", () => {
    const sources: FakeEventSource[] = []
    const factory: EventSourceFactory = () => { const source = new FakeEventSource(); sources.push(source); return source }
    const terminal = vi.fn()
    const { result, rerender } = renderHook(
      ({ runId }) => useExplainabilityStream(runId, "running", terminal, factory),
      { initialProps: { runId: "run-1" as string | null } },
    )
    act(() => {
      sources[0]?.emit(envelope(2, "future_graphloom_event"))
      sources[0]?.emit(envelope(1, "run_started"))
      sources[0]?.emit(envelope(2, "future_graphloom_event"))
    })
    expect(result.current.envelopes.map((value) => value.sequence)).toEqual([1, 2])
    rerender({ runId: "run-2" })
    expect(sources[0]?.closed).toBe(true)
    expect(result.current.envelopes).toEqual([])
  })

  it("stops EventSource reconnects after Store metadata becomes terminal", () => {
    const source = new FakeEventSource()
    const terminal = vi.fn()
    const factory: EventSourceFactory = () => source
    const { result, rerender } = renderHook(
      ({ status }) => useExplainabilityStream("run-1", status, terminal, factory),
      { initialProps: { status: "running" } },
    )
    act(() => source.onerror?.(new Event("error")))
    expect(result.current.status).toBe("reconnecting")
    rerender({ status: "completed" })
    expect(source.closed).toBe(true)
    expect(result.current.status).toBe("closed")
  })
})
