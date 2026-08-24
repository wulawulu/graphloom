import type { ExplainabilityEnvelope, ExplainabilityEventPayload } from "@/api/types"
import type { StudioTranslationKey } from "@/i18n/types"
import { latestContextSections } from "@/lib/context-evidence"

export type TimelineCategory = "lifecycle" | "retrieval" | "graph" | "context" | "llm" | "warning"

export interface TimelineDescriptor {
  labelKey: StudioTranslationKey | null
  rawLabel: string | null
  category: TimelineCategory
}

const knownEvents: Record<string, TimelineDescriptor> = {
  run_started: knownEvent("explainability.actions.runStarted", "lifecycle"),
  run_completed: knownEvent("explainability.actions.runCompleted", "lifecycle"),
  run_failed: knownEvent("explainability.actions.runFailed", "lifecycle"),
  query_started: knownEvent("explainability.labels.queryStarted", "lifecycle"),
  mapping_query_built: knownEvent("explainability.labels.mappingQueryBuilt", "retrieval"),
  embedding_started: knownEvent("explainability.labels.embeddingStarted", "retrieval"),
  embedding_completed: knownEvent("explainability.labels.embeddingCompleted", "retrieval"),
  candidates_retrieved: knownEvent("explainability.labels.candidatesRetrieved", "retrieval"),
  candidates_filtered: knownEvent("explainability.labels.candidatesFiltered", "retrieval"),
  entities_selected: knownEvent("explainability.labels.entitiesSelected", "graph"),
  graph_expansion_started: knownEvent("explainability.labels.graphExpansion", "graph"),
  relationships_selected: knownEvent("explainability.labels.relationshipsSelected", "graph"),
  community_reports_selected: knownEvent("explainability.labels.communityReportsSelected", "graph"),
  covariates_selected: knownEvent("explainability.labels.covariatesSelected", "graph"),
  text_units_selected: knownEvent("explainability.labels.textUnitsSelected", "context"),
  context_budget_allocated: knownEvent("explainability.labels.contextBudgetAllocated", "context"),
  context_section_built: knownEvent("explainability.labels.contextSectionBuilt", "context"),
  context_completed: knownEvent("explainability.labels.contextCompleted", "context"),
  global_context_built: knownEvent("explainability.labels.globalContextBuilt", "context"),
  global_map_started: knownEvent("explainability.labels.globalMapStarted", "llm"),
  global_map_batch_built: knownEvent("explainability.labels.globalMapBatchBuilt", "context"),
  global_map_points_produced: knownEvent("explainability.labels.globalMapPointsProduced", "llm"),
  global_reduce_context_built: knownEvent("explainability.labels.globalReduceContextBuilt", "context"),
  global_reduce_skipped: knownEvent("explainability.labels.globalReduceSkipped", "llm"),
  llm_request_started: knownEvent("explainability.labels.llmRequestStarted", "llm"),
  llm_request_completed: knownEvent("explainability.labels.llmRequestCompleted", "llm"),
  warning: knownEvent("explainability.labels.warning", "warning"),
}

function knownEvent(labelKey: StudioTranslationKey, category: TimelineCategory): TimelineDescriptor {
  return { labelKey, rawLabel: null, category }
}

export function describeEvent(event: ExplainabilityEventPayload): TimelineDescriptor {
  return knownEvents[event.type] ?? {
    labelKey: null,
    rawLabel: event.type.replaceAll("_", " "),
    category: "lifecycle",
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
