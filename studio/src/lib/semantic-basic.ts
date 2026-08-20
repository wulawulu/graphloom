import type {
  BasicRetrievalSkippedEvent,
  CandidatesFilteredEvent,
  CandidatesRetrievedEvent,
  ContextBudgetAllocatedEvent,
  ContextCompletedEvent,
  ContextSectionBuiltEvent,
  EmbeddingCompletedEvent,
  EmbeddingStartedEvent,
  ExplainabilityCandidate,
  ExplainabilityContextSection,
  ExplainabilityEnvelope,
  ExplainabilityEventPayload,
  LlmRequestCompletedEvent,
  LlmRequestStartedEvent,
} from "@/api/types"

export type BasicRetrievalStatus = "waiting" | "embedding" | "embedding_ready" | "retrieved" | "skipped"
export type BasicContextStatus = "waiting" | "assembling" | "completed"
export type BasicAnswerStatus = "waiting" | "generating" | "generated"

export interface BasicRetrievalSummary {
  status: BasicRetrievalStatus
  skippedReason: "empty_query" | null
  model?: string
  promptTokens?: number
  dimensions?: number
  elapsedMs?: number
  candidates: ExplainabilityCandidate[]
}

export interface BasicContextSummary {
  status: BasicContextStatus
  candidates: ExplainabilityCandidate[]
  candidateCount?: number
  selectedCount?: number
  tokenBudgetExcludedCount?: number
  tokenBudget?: number
  budgetedTokensUsed?: number
  truncated?: boolean
  selectedRecordIds: string[]
  exactContext: string | null
}

export interface BasicAnswerSummary {
  status: BasicAnswerStatus
  calls: number
  model?: string
  inputTokens?: number
  outputTokens?: number
  elapsedMs?: number
  exactPrompt: string | null
  rawResponse: string | null
}

export type BasicSemanticStep =
  | { id: "text-retrieval"; kind: "text-retrieval"; title: "Text Retrieval"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: BasicRetrievalSummary }
  | { id: "basic-context-assembly"; kind: "basic-context-assembly"; title: "Context Assembly"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: BasicContextSummary }
  | { id: "basic-answer-generation"; kind: "basic-answer-generation"; title: "Answer Generation"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: BasicAnswerSummary }

export interface BasicSemanticTimelineModel {
  steps: BasicSemanticStep[]
  diagnosticEvents: ExplainabilityEnvelope[]
}

interface TypedEnvelope<T extends ExplainabilityEventPayload> extends ExplainabilityEnvelope {
  record: ExplainabilityEnvelope["record"] & { event: T }
}

interface ContextLifecycle {
  rawEvents: ExplainabilityEnvelope[]
  budget?: TypedEnvelope<ContextBudgetAllocatedEvent>
  section?: TypedEnvelope<ContextSectionBuiltEvent>
  completed?: TypedEnvelope<ContextCompletedEvent>
}

export function isBasicSemanticStep(step: { kind: string }): step is BasicSemanticStep {
  return step.kind === "text-retrieval"
    || step.kind === "basic-context-assembly"
    || step.kind === "basic-answer-generation"
}

export function buildBasicSemanticTimeline(envelopes: readonly ExplainabilityEnvelope[]): BasicSemanticTimelineModel {
  const ordered = [...envelopes].sort(bySequence)
  const root = ordered.find((envelope) => envelope.record.parent_span_id === undefined
    && envelope.record.event.type === "query_started"
    && envelope.record.event.method === "basic")
  if (root === undefined) return { steps: [], diagnosticEvents: ordered }
  const rootSpan = root.record.span_id
  const terminal = ordered.find((envelope) => envelope.sequence > root.sequence
    && envelope.record.span_id === rootSpan
    && envelope.record.parent_span_id === undefined
    && (envelope.record.event.type === "run_completed" || envelope.record.event.type === "run_failed"))
  const terminalCutoff = terminal?.sequence ?? Number.MAX_SAFE_INTEGER

  const embeddingStarted: TypedEnvelope<EmbeddingStartedEvent>[] = []
  const embeddingCompleted: TypedEnvelope<EmbeddingCompletedEvent>[] = []
  const retrieved: TypedEnvelope<CandidatesRetrievedEvent>[] = []
  const filtered: TypedEnvelope<CandidatesFilteredEvent>[] = []
  const skipped: TypedEnvelope<BasicRetrievalSkippedEvent>[] = []
  const budgets: TypedEnvelope<ContextBudgetAllocatedEvent>[] = []
  const sections: TypedEnvelope<ContextSectionBuiltEvent>[] = []
  const contexts: TypedEnvelope<ContextCompletedEvent>[] = []
  const llmStarted: TypedEnvelope<LlmRequestStartedEvent>[] = []
  const llmCompleted: TypedEnvelope<LlmRequestCompletedEvent>[] = []

  for (const envelope of ordered) {
    if (envelope.sequence <= root.sequence
      || envelope.sequence >= terminalCutoff
      || envelope.record.parent_span_id !== rootSpan) continue
    const event = envelope.record.event
    if (isEmbeddingStarted(event)) embeddingStarted.push(typed(envelope, event))
    else if (isEmbeddingCompleted(event)) embeddingCompleted.push(typed(envelope, event))
    else if (isCandidatesRetrieved(event) && event.record_type === "text_unit") retrieved.push(typed(envelope, event))
    else if (isCandidatesFiltered(event) && event.record_type === "text_unit") filtered.push(typed(envelope, event))
    else if (isBasicRetrievalSkipped(event)) skipped.push(typed(envelope, event))
    else if (isContextBudgetAllocated(event)) budgets.push(typed(envelope, event))
    else if (isContextSectionBuilt(event) && event.section.section === "sources") sections.push(typed(envelope, event))
    else if (isContextCompleted(event)) contexts.push(typed(envelope, event))
    else if (isLlmRequestStarted(event)) llmStarted.push(typed(envelope, event))
    else if (isLlmRequestCompleted(event)) llmCompleted.push(typed(envelope, event))
  }

  const claimed = new Set<number>()
  const contextLifecycle = selectContextLifecycle(budgets, sections, contexts, filtered, skipped)
  contextLifecycle.rawEvents.forEach((event) => claimed.add(event.sequence))
  const contextCutoff = contextLifecycle.completed?.sequence ?? Number.MAX_SAFE_INTEGER
  const retrievalLifecycle = selectRetrievalLifecycle(
    retrieved,
    filtered,
    skipped,
    contextCutoff,
    contextLifecycle.section?.record.event.section,
  )
  retrievalLifecycle.rawEvents.forEach((event) => claimed.add(event.sequence))
  const retrievalStart = retrievalLifecycle.rawEvents[0]?.sequence
    ?? contextLifecycle.rawEvents[0]?.sequence
    ?? Number.MAX_SAFE_INTEGER
  const embeddingLifecycle = retrievalLifecycle.skipped === undefined
    ? selectPairLifecycle(embeddingStarted, embeddingCompleted, retrievalStart)
    : {}
  if (embeddingLifecycle.started !== undefined) claimed.add(embeddingLifecycle.started.sequence)
  if (embeddingLifecycle.completed !== undefined) claimed.add(embeddingLifecycle.completed.sequence)

  const llmLifecycle = selectPairLifecycle(
    llmStarted,
    llmCompleted,
    Number.MAX_SAFE_INTEGER,
    contextLifecycle.completed?.sequence,
  )
  if (llmLifecycle.started !== undefined) claimed.add(llmLifecycle.started.sequence)
  if (llmLifecycle.completed !== undefined) claimed.add(llmLifecycle.completed.sequence)

  const retrievalRaw = [embeddingLifecycle.started, embeddingLifecycle.completed, retrievalLifecycle.retrieved, retrievalLifecycle.skipped]
    .filter(defined)
    .sort(bySequence)
  const retrievalEvent = retrievalLifecycle.retrieved
  const embeddingStart = embeddingLifecycle.started
  const embeddingEnd = embeddingLifecycle.completed
  const retrievalSummary: BasicRetrievalSummary = {
    status: retrievalLifecycle.skipped !== undefined
      ? "skipped"
      : retrievalEvent !== undefined
        ? "retrieved"
        : embeddingEnd !== undefined
          ? "embedding_ready"
          : embeddingStart !== undefined
            ? "embedding"
            : "waiting",
    skippedReason: retrievalLifecycle.skipped?.record.event.reason ?? null,
    model: embeddingEnd?.record.event.model_id ?? embeddingStart?.record.event.model_id,
    promptTokens: embeddingEnd?.record.event.prompt_tokens,
    dimensions: embeddingEnd?.record.event.dimensions,
    elapsedMs: elapsedBetween(embeddingStart, embeddingEnd),
    candidates: retrievalEvent?.record.event.candidates ?? [],
  }

  const sourceSection = contextLifecycle.section?.record.event.section
  const decisions = retrievalLifecycle.filtered?.record.event.candidates ?? []
  const contextSummary: BasicContextSummary = {
    status: contextLifecycle.completed !== undefined
      ? "completed"
      : contextLifecycle.rawEvents.length > 0 || retrievalLifecycle.filtered !== undefined
        ? "assembling"
        : "waiting",
    candidates: decisions,
    candidateCount: sourceSection?.candidate_count,
    selectedCount: sourceSection?.selected_count,
    tokenBudgetExcludedCount: sourceSection === undefined
      ? undefined
      : decisions.filter((candidate) => !candidate.selected && candidate.reason === "token_budget").length,
    tokenBudget: sourceSection?.token_budget ?? contextLifecycle.budget?.record.event.total_token_budget,
    budgetedTokensUsed: sourceSection?.tokens_used,
    truncated: sourceSection?.truncated,
    selectedRecordIds: sourceSection?.selected_record_ids ?? [],
    exactContext: contextLifecycle.completed?.record.event.context ?? null,
  }

  const answerSummary: BasicAnswerSummary = {
    status: llmLifecycle.completed !== undefined ? "generated" : llmLifecycle.started !== undefined ? "generating" : "waiting",
    calls: Number(llmLifecycle.started !== undefined || llmLifecycle.completed !== undefined),
    model: llmLifecycle.completed?.record.event.model_id ?? llmLifecycle.started?.record.event.model_id,
    inputTokens: llmLifecycle.completed?.record.event.input_tokens ?? llmLifecycle.started?.record.event.prompt_tokens,
    outputTokens: llmLifecycle.completed?.record.event.output_tokens,
    elapsedMs: llmLifecycle.completed?.record.event.elapsed_ms,
    exactPrompt: llmLifecycle.started?.record.event.prompt ?? null,
    rawResponse: llmLifecycle.completed?.record.event.response ?? null,
  }
  const answerRaw = [llmLifecycle.started, llmLifecycle.completed]
    .filter(defined)
    .sort(bySequence)

  const contextRaw = [retrievalLifecycle.filtered, ...contextLifecycle.rawEvents]
    .filter(defined)
    .sort(bySequence)

  return {
    steps: [
      { id: "text-retrieval", kind: "text-retrieval", title: "Text Retrieval", rawEvents: retrievalRaw, focusEnvelope: null, summary: retrievalSummary },
      { id: "basic-context-assembly", kind: "basic-context-assembly", title: "Context Assembly", rawEvents: contextRaw, focusEnvelope: null, summary: contextSummary },
      { id: "basic-answer-generation", kind: "basic-answer-generation", title: "Answer Generation", rawEvents: answerRaw, focusEnvelope: null, summary: answerSummary },
    ],
    diagnosticEvents: ordered.filter((envelope) => !claimed.has(envelope.sequence)),
  }
}

function selectContextLifecycle(
  budgets: readonly TypedEnvelope<ContextBudgetAllocatedEvent>[],
  sections: readonly TypedEnvelope<ContextSectionBuiltEvent>[],
  contexts: readonly TypedEnvelope<ContextCompletedEvent>[],
  filtered: readonly TypedEnvelope<CandidatesFilteredEvent>[],
  skipped: readonly TypedEnvelope<BasicRetrievalSkippedEvent>[],
): ContextLifecycle {
  for (const completed of [...contexts].reverse()) {
    const span = completed.record.span_id
    const candidateSections = sections.filter((event) => event.record.span_id === span
      && event.sequence <= completed.sequence
      && hasCoherentRetrieval(event, filtered, skipped))
    for (const section of [...candidateSections].reverse()) {
      const budget = last(budgets.filter((event) => event.record.span_id === span
        && event.sequence <= section.sequence))
      if (budget !== undefined) {
        return { budget, section, completed, rawEvents: [budget, section, completed].sort(bySequence) }
      }
    }
  }
  for (const section of [...sections].reverse()) {
    if (!hasCoherentRetrieval(section, filtered, skipped)) continue
    const budget = last(budgets.filter((event) => event.record.span_id === section.record.span_id
      && event.sequence <= section.sequence))
    if (budget !== undefined) return { budget, section, rawEvents: [budget, section] }
  }
  const budget = last(budgets)
  return budget === undefined ? { rawEvents: [] } : { budget, rawEvents: [budget] }
}

function hasCoherentRetrieval(
  section: TypedEnvelope<ContextSectionBuiltEvent>,
  filtered: readonly TypedEnvelope<CandidatesFilteredEvent>[],
  skipped: readonly TypedEnvelope<BasicRetrievalSkippedEvent>[],
): boolean {
  const evidence = section.record.event.section
  const hasFilteredEvidence = filtered.some((event) => event.sequence <= section.sequence
    && matchesContextSection(event.record.event.candidates, evidence))
  return hasFilteredEvidence
    || (evidence.candidate_count === 0
      && skipped.some((event) => event.sequence <= section.sequence))
}

function selectRetrievalLifecycle(
  retrieved: readonly TypedEnvelope<CandidatesRetrievedEvent>[],
  filtered: readonly TypedEnvelope<CandidatesFilteredEvent>[],
  skipped: readonly TypedEnvelope<BasicRetrievalSkippedEvent>[],
  cutoff: number,
  expectedSection: ExplainabilityContextSection | undefined,
): { rawEvents: ExplainabilityEnvelope[]; retrieved?: TypedEnvelope<CandidatesRetrievedEvent>; filtered?: TypedEnvelope<CandidatesFilteredEvent>; skipped?: TypedEnvelope<BasicRetrievalSkippedEvent> } {
  const branches: Array<{ end: number; retrieved?: TypedEnvelope<CandidatesRetrievedEvent>; filtered?: TypedEnvelope<CandidatesFilteredEvent>; skipped?: TypedEnvelope<BasicRetrievalSkippedEvent> }> = []
  for (const decision of filtered) {
    if (decision.sequence > cutoff) continue
    if (expectedSection !== undefined && !matchesContextSection(decision.record.event.candidates, expectedSection)) continue
    const source = last(retrieved.filter((event) => event.record.span_id === decision.record.span_id && event.sequence <= decision.sequence))
    if (source !== undefined) branches.push({ end: decision.sequence, retrieved: source, filtered: decision })
  }
  for (const source of retrieved) if (source.sequence <= cutoff) branches.push({ end: source.sequence, retrieved: source })
  for (const skip of skipped) if (skip.sequence <= cutoff) branches.push({ end: skip.sequence, skipped: skip })
  const filteredBranches = branches.filter((branch) => branch.filtered !== undefined)
  const skipBranches = branches.filter((branch) => branch.skipped !== undefined)
  const branch = expectedSection?.candidate_count === 0 && skipBranches.length > 0
    ? skipBranches.sort((left, right) => left.end - right.end).at(-1)
    : filteredBranches.length > 0
      ? filteredBranches.sort((left, right) => left.end - right.end).at(-1)
      : branches.sort((left, right) => left.end - right.end).at(-1)
  if (branch === undefined) return { rawEvents: [] }
  return {
    ...branch,
    rawEvents: [branch.retrieved, branch.filtered, branch.skipped]
      .filter(defined)
      .sort(bySequence),
  }
}

function matchesContextSection(
  candidates: readonly ExplainabilityCandidate[],
  section: ExplainabilityContextSection,
): boolean {
  const selectedIds = candidates.filter((candidate) => candidate.selected).map((candidate) => candidate.id)
  return candidates.length === section.candidate_count
    && selectedIds.length === section.selected_count
    && selectedIds.length === section.selected_record_ids.length
    && selectedIds.every((id, index) => id === section.selected_record_ids[index])
}

function selectPairLifecycle<S extends ExplainabilityEventPayload, C extends ExplainabilityEventPayload>(
  starts: readonly TypedEnvelope<S>[],
  completions: readonly TypedEnvelope<C>[],
  cutoff: number,
  floor = Number.MIN_SAFE_INTEGER,
): { started?: TypedEnvelope<S>; completed?: TypedEnvelope<C> } {
  for (const completed of [...completions].reverse()) {
    if (completed.sequence > cutoff) continue
    const started = last(starts.filter((event) => event.record.span_id === completed.record.span_id
      && event.sequence >= floor
      && event.sequence <= completed.sequence))
    if (started !== undefined) return { started, completed }
  }
  const started = last(starts.filter((event) => event.sequence >= floor && event.sequence <= cutoff))
  return started === undefined ? {} : { started }
}

function elapsedBetween(started: ExplainabilityEnvelope | undefined, completed: ExplainabilityEnvelope | undefined): number | undefined {
  if (started === undefined || completed === undefined) return undefined
  const start = Date.parse(started.record.timestamp)
  const end = Date.parse(completed.record.timestamp)
  return Number.isFinite(start) && Number.isFinite(end) && end >= start ? end - start : undefined
}

function last<T>(values: readonly T[]): T | undefined {
  return values.at(-1)
}

function defined<T>(value: T | undefined): value is T {
  return value !== undefined
}

function bySequence(left: ExplainabilityEnvelope, right: ExplainabilityEnvelope): number {
  return left.sequence - right.sequence
}

function typed<T extends ExplainabilityEventPayload>(envelope: ExplainabilityEnvelope, event: T): TypedEnvelope<T> {
  return { ...envelope, record: { ...envelope.record, event } }
}

function isEmbeddingStarted(event: ExplainabilityEventPayload): event is EmbeddingStartedEvent {
  return event.type === "embedding_started"
    && isString(event.model_id)
    && isOptionalString(event.input)
}

function isEmbeddingCompleted(event: ExplainabilityEventPayload): event is EmbeddingCompletedEvent {
  return event.type === "embedding_completed"
    && isString(event.model_id)
    && isNonNegativeInteger(event.prompt_tokens)
    && isNonNegativeInteger(event.dimensions)
}

function isCandidatesRetrieved(event: ExplainabilityEventPayload): event is CandidatesRetrievedEvent {
  return event.type === "candidates_retrieved"
    && isString(event.record_type)
    && isCandidateArray(event.candidates)
}

function isCandidatesFiltered(event: ExplainabilityEventPayload): event is CandidatesFilteredEvent {
  return event.type === "candidates_filtered"
    && isString(event.record_type)
    && isCandidateArray(event.candidates)
}
function isBasicRetrievalSkipped(event: ExplainabilityEventPayload): event is BasicRetrievalSkippedEvent { return event.type === "basic_retrieval_skipped" && event.reason === "empty_query" }
function isContextBudgetAllocated(event: ExplainabilityEventPayload): event is ContextBudgetAllocatedEvent {
  return event.type === "context_budget_allocated"
    && isNonNegativeInteger(event.total_token_budget)
    && Array.isArray(event.sections)
    && event.sections.every((section) => isObject(section)
      && isString(section.section)
      && isNonNegativeInteger(section.token_budget))
}

function isContextSectionBuilt(event: ExplainabilityEventPayload): event is ContextSectionBuiltEvent {
  return event.type === "context_section_built" && isContextSection(event.section)
}

function isContextCompleted(event: ExplainabilityEventPayload): event is ContextCompletedEvent {
  return event.type === "context_completed"
    && isNonNegativeInteger(event.tokens_used)
    && isOptionalString(event.context)
}

function isLlmRequestStarted(event: ExplainabilityEventPayload): event is LlmRequestStartedEvent {
  return event.type === "llm_request_started"
    && isString(event.model_id)
    && isNonNegativeInteger(event.prompt_tokens)
    && isOptionalString(event.prompt)
}

function isLlmRequestCompleted(event: ExplainabilityEventPayload): event is LlmRequestCompletedEvent {
  return event.type === "llm_request_completed"
    && isString(event.model_id)
    && isNonNegativeInteger(event.input_tokens)
    && isNonNegativeInteger(event.output_tokens)
    && isNonNegativeInteger(event.elapsed_ms)
    && isOptionalString(event.response)
}

function isCandidateArray(value: unknown): value is ExplainabilityCandidate[] {
  return Array.isArray(value) && value.every((candidate) => isObject(candidate)
    && isString(candidate.id)
    && isString(candidate.record_type)
    && typeof candidate.selected === "boolean"
    && isOptionalString(candidate.short_id)
    && (candidate.score === undefined || isFiniteNumber(candidate.score))
    && (candidate.rank === undefined || isPositiveInteger(candidate.rank))
    && isOptionalString(candidate.reason))
}

function isContextSection(value: unknown): value is ExplainabilityContextSection {
  return isObject(value)
    && isString(value.section)
    && isNonNegativeInteger(value.token_budget)
    && isNonNegativeInteger(value.tokens_used)
    && isNonNegativeInteger(value.candidate_count)
    && isNonNegativeInteger(value.selected_count)
    && typeof value.truncated === "boolean"
    && Array.isArray(value.selected_record_ids)
    && value.selected_record_ids.every(isString)
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

function isString(value: unknown): value is string {
  return typeof value === "string"
}

function isOptionalString(value: unknown): value is string | undefined {
  return value === undefined || isString(value)
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value)
}

function isNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && typeof value === "number" && value >= 0
}

function isPositiveInteger(value: unknown): value is number {
  return isNonNegativeInteger(value) && value > 0
}
