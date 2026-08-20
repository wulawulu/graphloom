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
  exactRenderedTokensUsed?: number
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
  const rootSpan = ordered.find((envelope) => envelope.record.parent_span_id === undefined
    && envelope.record.event.type === "query_started"
    && envelope.record.event.method === "basic")?.record.span_id
  if (rootSpan === undefined) return { steps: [], diagnosticEvents: ordered }

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
    if (envelope.record.parent_span_id !== rootSpan) continue
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
  const contextLifecycle = selectContextLifecycle(budgets, sections, contexts)
  contextLifecycle.rawEvents.forEach((event) => claimed.add(event.sequence))
  const contextCutoff = contextLifecycle.completed?.sequence ?? Number.MAX_SAFE_INTEGER
  const retrievalLifecycle = selectRetrievalLifecycle(
    retrieved,
    filtered,
    skipped,
    contextCutoff,
    contextLifecycle.section?.record.event.section.candidate_count,
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
    exactRenderedTokensUsed: contextLifecycle.completed?.record.event.tokens_used,
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
): ContextLifecycle {
  for (const completed of [...contexts].reverse()) {
    const span = completed.record.span_id
    const budget = last(budgets.filter((event) => event.record.span_id === span && event.sequence <= completed.sequence))
    const section = last(sections.filter((event) => event.record.span_id === span && event.sequence <= completed.sequence))
    if (budget !== undefined && section !== undefined) {
      return { budget, section, completed, rawEvents: [budget, section, completed].sort(bySequence) }
    }
  }
  const progressive = [...budgets, ...sections].sort(bySequence)
  const latest = progressive.at(-1)
  if (latest === undefined) return { rawEvents: [] }
  const span = latest.record.span_id
  const budget = last(budgets.filter((event) => event.record.span_id === span && event.sequence <= latest.sequence))
  const section = last(sections.filter((event) => event.record.span_id === span && event.sequence <= latest.sequence))
  return { budget, section, rawEvents: [budget, section].filter(defined).sort(bySequence) }
}

function selectRetrievalLifecycle(
  retrieved: readonly TypedEnvelope<CandidatesRetrievedEvent>[],
  filtered: readonly TypedEnvelope<CandidatesFilteredEvent>[],
  skipped: readonly TypedEnvelope<BasicRetrievalSkippedEvent>[],
  cutoff: number,
  expectedCandidateCount: number | undefined,
): { rawEvents: ExplainabilityEnvelope[]; retrieved?: TypedEnvelope<CandidatesRetrievedEvent>; filtered?: TypedEnvelope<CandidatesFilteredEvent>; skipped?: TypedEnvelope<BasicRetrievalSkippedEvent> } {
  const branches: Array<{ end: number; retrieved?: TypedEnvelope<CandidatesRetrievedEvent>; filtered?: TypedEnvelope<CandidatesFilteredEvent>; skipped?: TypedEnvelope<BasicRetrievalSkippedEvent> }> = []
  for (const decision of filtered) {
    if (decision.sequence > cutoff) continue
    const source = last(retrieved.filter((event) => event.record.span_id === decision.record.span_id && event.sequence <= decision.sequence))
    if (source !== undefined) branches.push({ end: decision.sequence, retrieved: source, filtered: decision })
  }
  for (const source of retrieved) if (source.sequence <= cutoff) branches.push({ end: source.sequence, retrieved: source })
  for (const skip of skipped) if (skip.sequence <= cutoff) branches.push({ end: skip.sequence, skipped: skip })
  const filteredBranches = branches.filter((branch) => branch.filtered !== undefined)
  const skipBranches = branches.filter((branch) => branch.skipped !== undefined)
  const branch = expectedCandidateCount === 0 && skipBranches.length > 0
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

function isEmbeddingStarted(event: ExplainabilityEventPayload): event is EmbeddingStartedEvent { return event.type === "embedding_started" }
function isEmbeddingCompleted(event: ExplainabilityEventPayload): event is EmbeddingCompletedEvent { return event.type === "embedding_completed" }
function isCandidatesRetrieved(event: ExplainabilityEventPayload): event is CandidatesRetrievedEvent { return event.type === "candidates_retrieved" }
function isCandidatesFiltered(event: ExplainabilityEventPayload): event is CandidatesFilteredEvent { return event.type === "candidates_filtered" }
function isBasicRetrievalSkipped(event: ExplainabilityEventPayload): event is BasicRetrievalSkippedEvent { return event.type === "basic_retrieval_skipped" && event.reason === "empty_query" }
function isContextBudgetAllocated(event: ExplainabilityEventPayload): event is ContextBudgetAllocatedEvent { return event.type === "context_budget_allocated" }
function isContextSectionBuilt(event: ExplainabilityEventPayload): event is ContextSectionBuiltEvent { return event.type === "context_section_built" }
function isContextCompleted(event: ExplainabilityEventPayload): event is ContextCompletedEvent { return event.type === "context_completed" }
function isLlmRequestStarted(event: ExplainabilityEventPayload): event is LlmRequestStartedEvent { return event.type === "llm_request_started" }
function isLlmRequestCompleted(event: ExplainabilityEventPayload): event is LlmRequestCompletedEvent { return event.type === "llm_request_completed" }
