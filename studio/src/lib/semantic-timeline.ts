import type { ExplainabilityCandidate, ExplainabilityContextSection, ExplainabilityEnvelope } from "@/api/types"
import { latestContextSections } from "@/lib/context-evidence"

export type SemanticStepKind = "entity-mapping" | "graph-expansion" | "context-assembly" | "answer-generation"
export type FinalContextStatus = "included" | "excluded" | "unknown"
export type SelectionStatus = "pending" | "selected" | "excluded"

export interface ExplainabilityRecordView {
  stableId: string
  shortId?: string
  title?: string
  recordType: string
  score?: number
  rank?: number
  selected: boolean
  reason?: string
  sourceId?: string
  relationshipId?: string
  expansionDepth?: number
  selectionStatus: SelectionStatus
  finalContext: FinalContextStatus
}

export interface EntityMappingSummary {
  candidates: ExplainabilityRecordView[]
  retrievedCount: number
  selectedCount: number
  excludedCount: number
  pendingCount: number
  model?: string
  promptTokens?: number
  dimensions?: number
  elapsedMs?: number
}

export interface GraphExpansionSummary {
  records: ExplainabilityRecordView[]
  selectedCounts: Record<string, number>
}

export type ContextSectionView = ExplainabilityContextSection

export interface ContextAssemblySummary {
  sections: ContextSectionView[]
  totalTokenBudget?: number
  tokensUsed?: number
  exactContext: string | null
}

export interface AnswerGenerationSummary {
  calls: number
  inputTokens: number
  outputTokens: number
  elapsedMs: number
  model?: string
}

export type SemanticStep =
  | { id: "entity-mapping"; kind: "entity-mapping"; title: "Entity Mapping"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: ExplainabilityEnvelope | null; summary: EntityMappingSummary }
  | { id: "graph-expansion"; kind: "graph-expansion"; title: "Graph Expansion"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: ExplainabilityEnvelope | null; summary: GraphExpansionSummary }
  | { id: "context-assembly"; kind: "context-assembly"; title: "Context Assembly"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: ContextAssemblySummary }
  | { id: "answer-generation"; kind: "answer-generation"; title: "Answer Generation"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: AnswerGenerationSummary }

export interface SemanticTimelineModel {
  steps: SemanticStep[]
  diagnosticEvents: ExplainabilityEnvelope[]
}

const ENTITY_MAPPING_EVENTS = new Set(["mapping_query_built", "embedding_started", "embedding_completed", "candidates_retrieved", "candidates_filtered", "entities_selected"])
const GRAPH_EXPANSION_EVENTS = new Set(["graph_expansion_started", "relationships_selected", "community_reports_selected", "covariates_selected", "text_units_selected"])
const CONTEXT_ASSEMBLY_EVENTS = new Set(["context_budget_allocated", "context_section_built", "context_completed"])
const ANSWER_GENERATION_EVENTS = new Set(["llm_request_started", "llm_request_completed"])

const SELECTION_FIELDS: Record<string, string> = {
  entities_selected: "entities",
  relationships_selected: "relationships",
  community_reports_selected: "community_reports",
  covariates_selected: "covariates",
  text_units_selected: "text_units",
}

const CONTEXT_SECTION_BY_RECORD_TYPE: Record<string, string> = {
  entity: "entities",
  relationship: "relationships",
  community_report: "community_reports",
  text_unit: "sources",
  covariate: "covariates",
}

export function buildSemanticTimeline(envelopes: readonly ExplainabilityEnvelope[]): SemanticTimelineModel {
  const ordered = [...envelopes].sort((left, right) => left.sequence - right.sequence)
  const grouped = {
    entityMapping: [] as ExplainabilityEnvelope[],
    graphExpansion: [] as ExplainabilityEnvelope[],
    contextAssembly: [] as ExplainabilityEnvelope[],
    answerGeneration: [] as ExplainabilityEnvelope[],
    diagnostics: [] as ExplainabilityEnvelope[],
  }
  const contextSections = latestContextSections(ordered)

  for (const envelope of ordered) {
    const event = envelope.record.event
    if (ENTITY_MAPPING_EVENTS.has(event.type)) grouped.entityMapping.push(envelope)
    else if (GRAPH_EXPANSION_EVENTS.has(event.type)) grouped.graphExpansion.push(envelope)
    else if (CONTEXT_ASSEMBLY_EVENTS.has(event.type)) grouped.contextAssembly.push(envelope)
    else if (ANSWER_GENERATION_EVENTS.has(event.type)) grouped.answerGeneration.push(envelope)
    else grouped.diagnostics.push(envelope)

  }

  const finalContext = buildFinalContextIndex(contextSections)
  const steps: SemanticStep[] = []
  if (grouped.entityMapping.length > 0) steps.push(buildEntityMapping(grouped.entityMapping, finalContext))
  if (grouped.graphExpansion.length > 0) steps.push(buildGraphExpansion(grouped.graphExpansion, finalContext))
  if (grouped.contextAssembly.length > 0) steps.push(buildContextAssembly(grouped.contextAssembly, contextSections))
  if (grouped.answerGeneration.length > 0) steps.push(buildAnswerGeneration(grouped.answerGeneration))
  return { steps, diagnosticEvents: grouped.diagnostics }
}

function buildEntityMapping(rawEvents: ExplainabilityEnvelope[], finalContext: FinalContextIndex): SemanticStep {
  const candidates = new Map<string, CandidateAccumulator>()
  const retrieved = new Set<string>()
  let focusEnvelope: ExplainabilityEnvelope | null = null
  let model: string | undefined
  let promptTokens: number | undefined
  let dimensions: number | undefined
  let embeddingStartedAt: number | undefined
  let embeddingCompletedAt: number | undefined

  for (const envelope of rawEvents) {
    const event = envelope.record.event
    if (event.type === "candidates_retrieved") {
      const retrievedCandidates = asCandidates(event.candidates)
      for (const candidate of retrievedCandidates) retrieved.add(candidateKey(candidate))
      mergeCandidateDecisions(candidates, retrievedCandidates, "pending")
    }
    if (event.type === "candidates_filtered") {
      const filtered = asCandidates(event.candidates)
      mergeCandidateDecisions(candidates, filtered)
    }
    if (event.type === "entities_selected") {
      const selected = asCandidates(event.entities)
      mergeCandidateDecisions(candidates, selected)
    }
    if (event.type === "entities_selected" && asCandidates(event.entities).some((candidate) => candidate.selected)) focusEnvelope = envelope
    if (event.type === "embedding_started") {
      model = stringValue(event.model_id) ?? model
      embeddingStartedAt = timestamp(envelope) ?? embeddingStartedAt
    }
    if (event.type === "embedding_completed") {
      model = stringValue(event.model_id) ?? model
      promptTokens = numberValue(event.prompt_tokens) ?? promptTokens
      dimensions = numberValue(event.dimensions) ?? dimensions
      embeddingCompletedAt = timestamp(envelope) ?? embeddingCompletedAt
    }
  }

  const views = [...candidates.values()].map(({ candidate, selectionStatus }) => candidateView(candidate, finalContext, selectionStatus))
  const selectedCount = views.filter((candidate) => candidate.selectionStatus === "selected").length
  const excludedCount = views.filter((candidate) => candidate.selectionStatus === "excluded").length
  const pendingCount = views.filter((candidate) => candidate.selectionStatus === "pending").length
  const elapsedMs = embeddingStartedAt !== undefined && embeddingCompletedAt !== undefined && embeddingCompletedAt >= embeddingStartedAt
    ? embeddingCompletedAt - embeddingStartedAt
    : undefined
  return {
    id: "entity-mapping",
    kind: "entity-mapping",
    title: "Entity Mapping",
    rawEvents,
    focusEnvelope,
    summary: {
      candidates: views,
      retrievedCount: retrieved.size > 0 ? retrieved.size : views.length,
      selectedCount,
      excludedCount,
      pendingCount,
      model,
      promptTokens,
      dimensions,
      elapsedMs,
    },
  }
}

function buildGraphExpansion(rawEvents: ExplainabilityEnvelope[], finalContext: FinalContextIndex): SemanticStep {
  const candidates = new Map<string, ExplainabilityCandidate>()
  let focusEnvelope: ExplainabilityEnvelope | null = null
  for (const envelope of rawEvents) {
    const event = envelope.record.event
    const field = SELECTION_FIELDS[event.type]
    if (field !== undefined) mergeCandidates(candidates, asCandidates(event[field]))
    if (event.type === "relationships_selected" && asCandidates(event.relationships).some((candidate) => candidate.selected)) focusEnvelope = envelope
    else if (focusEnvelope === null && event.type === "graph_expansion_started") focusEnvelope = envelope
  }
  const records = [...candidates.values()].map((candidate) => candidateView(candidate, finalContext, candidate.selected ? "selected" : "excluded"))
  const selectedCounts: Record<string, number> = {}
  for (const record of records) {
    if (!record.selected) continue
    selectedCounts[record.recordType] = (selectedCounts[record.recordType] ?? 0) + 1
  }
  return { id: "graph-expansion", kind: "graph-expansion", title: "Graph Expansion", rawEvents, focusEnvelope, summary: { records, selectedCounts } }
}

function buildContextAssembly(rawEvents: ExplainabilityEnvelope[], sections: ExplainabilityContextSection[]): SemanticStep {
  let totalTokenBudget: number | undefined
  let tokensUsed: number | undefined
  let exactContext: string | null = null
  for (const envelope of rawEvents) {
    const event = envelope.record.event
    if (event.type === "context_budget_allocated") totalTokenBudget = numberValue(event.total_token_budget) ?? totalTokenBudget
    if (event.type === "context_completed") {
      tokensUsed = numberValue(event.tokens_used) ?? tokensUsed
      exactContext = stringValue(event.context) ?? null
    }
  }
  if (tokensUsed === undefined && sections.length > 0) tokensUsed = sections.reduce((total, section) => total + section.tokens_used, 0)
  return { id: "context-assembly", kind: "context-assembly", title: "Context Assembly", rawEvents, focusEnvelope: null, summary: { sections, totalTokenBudget, tokensUsed, exactContext } }
}

function buildAnswerGeneration(rawEvents: ExplainabilityEnvelope[]): SemanticStep {
  let startedCalls = 0
  let completedCalls = 0
  let startedInputTokens = 0
  let inputTokens = 0
  let outputTokens = 0
  let elapsedMs = 0
  let model: string | undefined
  for (const envelope of rawEvents) {
    const event = envelope.record.event
    if (event.type === "llm_request_started") {
      startedCalls += 1
      startedInputTokens += numberValue(event.prompt_tokens) ?? 0
    }
    if (event.type === "llm_request_completed") {
      completedCalls += 1
      inputTokens += numberValue(event.input_tokens) ?? 0
      outputTokens += numberValue(event.output_tokens) ?? 0
      elapsedMs += numberValue(event.elapsed_ms) ?? 0
    }
    model = stringValue(event.model_id) ?? model
  }
  return { id: "answer-generation", kind: "answer-generation", title: "Answer Generation", rawEvents, focusEnvelope: null, summary: { calls: Math.max(startedCalls, completedCalls), inputTokens: completedCalls > 0 ? inputTokens : startedInputTokens, outputTokens, elapsedMs, model } }
}

interface FinalContextIndex {
  sectionIds: Map<string, Set<string>>
  knownSections: Set<string>
}

function buildFinalContextIndex(sections: readonly ExplainabilityContextSection[]): FinalContextIndex {
  const sectionIds = new Map<string, Set<string>>()
  const knownSections = new Set<string>()
  for (const section of sections) {
    knownSections.add(section.section)
    const ids = sectionIds.get(section.section) ?? new Set<string>()
    section.selected_record_ids.forEach((id) => ids.add(id))
    sectionIds.set(section.section, ids)
  }
  return { sectionIds, knownSections }
}

function candidateView(candidate: ExplainabilityCandidate, finalContext: FinalContextIndex, selectionStatus: SelectionStatus): ExplainabilityRecordView {
  const section = CONTEXT_SECTION_BY_RECORD_TYPE[candidate.record_type]
  const finalContextStatus: FinalContextStatus = section === undefined || !finalContext.knownSections.has(section)
    ? "unknown"
    : finalContext.sectionIds.get(section)?.has(candidate.id) === true
      ? "included"
      : "excluded"
  return {
    stableId: candidate.id,
    shortId: candidate.short_id,
    title: candidate.title,
    recordType: candidate.record_type,
    score: candidate.score,
    rank: candidate.rank,
    selected: candidate.selected,
    reason: candidate.reason,
    sourceId: candidate.source_id,
    relationshipId: candidate.relationship_id,
    expansionDepth: candidate.expansion_depth,
    selectionStatus,
    finalContext: finalContextStatus,
  }
}

interface CandidateAccumulator {
  candidate: ExplainabilityCandidate
  selectionStatus: SelectionStatus
}

function mergeCandidateDecisions(
  target: Map<string, CandidateAccumulator>,
  candidates: readonly ExplainabilityCandidate[],
  status?: SelectionStatus,
): void {
  for (const candidate of candidates) {
    const key = candidateKey(candidate)
    const current = target.get(key)
    const incomingStatus = status ?? (candidate.selected ? "selected" : "excluded")
    const preserveDecision = incomingStatus === "pending" && current !== undefined && current.selectionStatus !== "pending"
    target.set(key, {
      candidate: preserveDecision
        ? { ...current.candidate, ...candidate, selected: current.candidate.selected, reason: current.candidate.reason }
        : { ...current?.candidate, ...candidate },
      selectionStatus: preserveDecision ? current.selectionStatus : incomingStatus,
    })
  }
}

function mergeCandidates(target: Map<string, ExplainabilityCandidate>, candidates: readonly ExplainabilityCandidate[]): void {
  for (const candidate of candidates) {
    const key = candidateKey(candidate)
    target.set(key, { ...target.get(key), ...candidate })
  }
}

function candidateKey(candidate: ExplainabilityCandidate): string {
  return `${candidate.record_type}:${candidate.id}`
}

function asCandidates(value: unknown): ExplainabilityCandidate[] {
  if (!Array.isArray(value)) return []
  return value.filter((candidate): candidate is ExplainabilityCandidate => {
    if (typeof candidate !== "object" || candidate === null) return false
    const value = candidate as Partial<ExplainabilityCandidate>
    return typeof value.id === "string" && value.id.length > 0 && typeof value.record_type === "string" && typeof value.selected === "boolean"
  })
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined
}

function timestamp(envelope: ExplainabilityEnvelope): number | undefined {
  const value = Date.parse(envelope.record.timestamp)
  return Number.isNaN(value) ? undefined : value
}
