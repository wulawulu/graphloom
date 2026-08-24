import type { ExplainabilityEnvelope } from "@/api/types"
import { describeEvent } from "@/lib/explainability"

export function detectRunContentMode(envelopes: readonly ExplainabilityEnvelope[]): "metadata" | "content" | "debug" | null {
  const started = [...envelopes]
    .sort((left, right) => left.sequence - right.sequence)
    .find((envelope) => envelope.record.parent_span_id === undefined && envelope.record.event.type === "run_started")
  const mode = started?.record.event.content_mode
  return mode === "metadata" || mode === "content" || mode === "debug" ? mode : null
}

export function classifyTimelineResidualEvents(envelopes: readonly ExplainabilityEnvelope[]): { warnings: ExplainabilityEnvelope[]; developerEvents: ExplainabilityEnvelope[] } {
  const warnings = envelopes.filter((envelope) => {
    const event = envelope.record.event
    return event.type === "warning" || event.type === "run_failed" || describeEvent(event).category !== "lifecycle"
  })
  const warningSequences = new Set(warnings.map((envelope) => envelope.sequence))
  return { warnings, developerEvents: envelopes.filter((envelope) => !warningSequences.has(envelope.sequence)) }
}
