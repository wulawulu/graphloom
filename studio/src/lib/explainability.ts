import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import { latestContextSections } from "@/lib/context-evidence"

export type TimelineCategory = "Lifecycle" | "Retrieval" | "Graph" | "Context" | "LLM" | "Warning"

export interface TimelineDescriptor {
  label: string
  category: TimelineCategory
}

const knownEvents: Record<string, TimelineDescriptor> = {
  run_started: { label: "Run started", category: "Lifecycle" },
  run_completed: { label: "Run completed", category: "Lifecycle" },
  run_failed: { label: "Run failed", category: "Lifecycle" },
  query_started: { label: "Query started", category: "Lifecycle" },
  mapping_query_built: { label: "Mapping query built", category: "Retrieval" },
  embedding_started: { label: "Embedding started", category: "Retrieval" },
  embedding_completed: { label: "Embedding completed", category: "Retrieval" },
  candidates_retrieved: { label: "Candidates retrieved", category: "Retrieval" },
  candidates_filtered: { label: "Candidates filtered", category: "Retrieval" },
  entities_selected: { label: "Entities selected", category: "Graph" },
  graph_expansion_started: { label: "Graph expansion", category: "Graph" },
  relationships_selected: { label: "Relationships selected", category: "Graph" },
  community_reports_selected: { label: "Community reports selected", category: "Graph" },
  covariates_selected: { label: "Covariates selected", category: "Graph" },
  text_units_selected: { label: "Text units selected", category: "Context" },
  context_budget_allocated: { label: "Context budget allocated", category: "Context" },
  context_section_built: { label: "Context section built", category: "Context" },
  context_completed: { label: "Context completed", category: "Context" },
  global_context_built: { label: "Global context built", category: "Context" },
  global_map_started: { label: "Global Map started", category: "LLM" },
  global_map_batch_built: { label: "Global Map batch built", category: "Context" },
  global_map_points_produced: { label: "Global Map points produced", category: "LLM" },
  global_reduce_context_built: { label: "Global Reduce context built", category: "Context" },
  global_reduce_skipped: { label: "Global Reduce skipped", category: "LLM" },
  llm_request_started: { label: "LLM request started", category: "LLM" },
  llm_request_completed: { label: "LLM request completed", category: "LLM" },
  warning: { label: "Warning", category: "Warning" },
}

export function describeEvent(event: ExplainabilityEventPayload): TimelineDescriptor {
  return knownEvents[event.type] ?? {
    label: event.type.replaceAll("_", " "),
    category: "Lifecycle",
  }
}

export function isTerminalEvent(event: ExplainabilityEventPayload): boolean {
  return event.type === "run_completed" || event.type === "run_failed"
}

export function mergeEnvelopes(
  current: readonly ExplainabilityEnvelope[],
  incoming: ExplainabilityEnvelope,
): ExplainabilityEnvelope[] {
  if (current.some((value) => value.sequence === incoming.sequence)) {
    return [...current]
  }
  return [...current, incoming].sort((left, right) => left.sequence - right.sequence)
}

interface CandidateLike { id?: unknown; selected?: unknown }

function candidateIds(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.flatMap((candidate: unknown) => {
    if (typeof candidate !== "object" || candidate === null) return []
    const { id, selected } = candidate as CandidateLike
    return typeof id === "string" && selected === true ? [id] : []
  })
}

export interface GraphHighlight {
  entityIds: string[]
  relationshipIds: string[]
}

export function deriveFinalGraphFocus(envelopes: readonly ExplainabilityEnvelope[]): GraphHighlight | null {
  const focus: GraphHighlight = { entityIds: [], relationshipIds: [] }
  const seen = { entities: new Set<string>(), relationships: new Set<string>() }

  for (const section of latestContextSections(envelopes)) {
    if (section.section === "entities") {
      appendFirstSeen(focus.entityIds, seen.entities, section.selected_record_ids)
    } else if (section.section === "relationships") {
      appendFirstSeen(focus.relationshipIds, seen.relationships, section.selected_record_ids)
    }
  }

  return nonEmptyHighlight(focus)
}

function appendFirstSeen(target: string[], seen: Set<string>, values: readonly string[]): void {
  for (const value of values) {
    if (seen.has(value)) continue
    seen.add(value)
    target.push(value)
  }
}

function nonEmptyHighlight(highlight: GraphHighlight): GraphHighlight | null {
  return highlight.entityIds.length === 0 && highlight.relationshipIds.length === 0 ? null : highlight
}

export function highlightFromEvent(event: ExplainabilityEventPayload): GraphHighlight | null {
  if (event.type === "entities_selected") {
    return nonEmptyHighlight({ entityIds: candidateIds(event.entities), relationshipIds: [] })
  }
  if (event.type === "relationships_selected") {
    return nonEmptyHighlight({ entityIds: [], relationshipIds: candidateIds(event.relationships) })
  }
  if (event.type === "graph_expansion_started" && Array.isArray(event.seed_entity_ids)) {
    return nonEmptyHighlight({
      entityIds: event.seed_entity_ids.filter((value): value is string => typeof value === "string"),
      relationshipIds: [],
    })
  }
  return null
}

export function eventSummary(event: ExplainabilityEventPayload): string {
  const numeric = (name: string): string | null =>
    typeof event[name] === "number" ? `${name.replaceAll("_", " ")}: ${String(event[name])}` : null
  const model = typeof event.model_id === "string" ? `model: ${event.model_id}` : null
  const section = typeof event.section === "string"
    ? `section: ${event.section}`
    : typeof event.section === "object" && event.section !== null && "section" in event.section
      ? `section: ${String(event.section.section)}`
      : null
  const candidates = ["entities", "relationships", "community_reports", "covariates", "text_units", "candidates"]
    .map((key) => Array.isArray(event[key]) ? `${key.replaceAll("_", " ")}: ${String(event[key].length)}` : null)
    .find((value) => value !== null)
  return [model, section, candidates, numeric("elapsed_ms"), numeric("tokens_used"), numeric("input_tokens"), numeric("output_tokens")]
    .filter((value): value is string => value !== null)
    .join(" · ")
}
