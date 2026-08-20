import type {
  ExplainabilityEnvelope,
  ExplainabilityEventPayload,
  GlobalContextBuiltEvent,
  GlobalMapBatchBuiltEvent,
  GlobalMapPointDecision,
  GlobalMapPointEvidence,
  GlobalMapPointsProducedEvent,
  GlobalMapStartedEvent,
  GlobalReduceContextBuiltEvent,
  GlobalReduceSkippedEvent,
  LlmRequestCompletedEvent,
  LlmRequestStartedEvent,
} from "@/api/types"

export type GlobalMapBatchStatus = "ready" | "analyzing" | "response_received" | "completed"

export interface GlobalMapPointView extends GlobalMapPointEvidence {
  identity: string
}

export interface GlobalReduceDecisionView extends GlobalMapPointDecision {
  identity: string
}

export interface GlobalMapBatchView {
  batchIndex: number
  spanId: string
  reportCount: number
  reportIds: string[]
  tokensUsed: number
  tokenBudget: number
  exactContext: string | null
  status: GlobalMapBatchStatus
  model?: string
  inputTokens?: number
  outputTokens?: number
  elapsedMs?: number
  exactPrompt: string | null
  rawResponse: string | null
  points: GlobalMapPointView[]
  rawEvents: ExplainabilityEnvelope[]
}

export interface GlobalCommunityContextSummary {
  built: boolean
  reportCount?: number
  batchCount?: number
  tokensUsed: number
  batches: GlobalMapBatchView[]
}

export interface GlobalMapAnalysisSummary {
  started: boolean
  batchCount?: number
  analystCalls: number
  pointCount: number
  positivePointCount: number
  batches: GlobalMapBatchView[]
}

export interface GlobalReduceSummary {
  built: boolean
  candidatePointCount?: number
  positivePointCount?: number
  selectedPointCount?: number
  nonPositiveCount?: number
  tokenBudgetExcludedCount?: number
  tokenBudget?: number
  tokensUsed?: number
  truncated: boolean
  decisions: GlobalReduceDecisionView[]
  exactContext: string | null
  skippedReason: "no_positive_points" | null
}

export interface GlobalAnswerGenerationSummary {
  calls: number
  generated: boolean
  noDataAnswer: boolean
  model?: string
  inputTokens?: number
  outputTokens?: number
  elapsedMs?: number
  exactPrompt: string | null
  rawResponse: string | null
}

export type GlobalSemanticStep =
  | { id: "community-context"; kind: "community-context"; title: "Community Context"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalCommunityContextSummary }
  | { id: "map-analysis"; kind: "map-analysis"; title: "Map Analysis"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalMapAnalysisSummary }
  | { id: "evidence-reduction"; kind: "evidence-reduction"; title: "Evidence Reduction"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalReduceSummary }
  | { id: "global-answer-generation"; kind: "global-answer-generation"; title: "Answer Generation"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalAnswerGenerationSummary }

export interface GlobalSemanticTimelineModel {
  steps: GlobalSemanticStep[]
  diagnosticEvents: ExplainabilityEnvelope[]
}

export function isGlobalSemanticStep(step: { kind: string }): step is GlobalSemanticStep {
  return step.kind === "community-context"
    || step.kind === "map-analysis"
    || step.kind === "evidence-reduction"
    || step.kind === "global-answer-generation"
}

interface BatchEvidence {
  latestBuilt: ExplainabilityEnvelope & { record: { event: GlobalMapBatchBuiltEvent } }
  rawEvents: ExplainabilityEnvelope[]
  started?: ExplainabilityEnvelope & { record: { event: LlmRequestStartedEvent } }
  completed?: ExplainabilityEnvelope & { record: { event: LlmRequestCompletedEvent } }
  produced?: ExplainabilityEnvelope & { record: { event: GlobalMapPointsProducedEvent } }
}

export function buildGlobalSemanticTimeline(ordered: readonly ExplainabilityEnvelope[]): GlobalSemanticTimelineModel {
  let contextBuilt: (ExplainabilityEnvelope & { record: { event: GlobalContextBuiltEvent } }) | undefined
  let mapStarted: (ExplainabilityEnvelope & { record: { event: GlobalMapStartedEvent } }) | undefined
  let reduceBuilt: (ExplainabilityEnvelope & { record: { event: GlobalReduceContextBuiltEvent } }) | undefined
  const reduceSkippedEvents: Array<ExplainabilityEnvelope & { record: { event: GlobalReduceSkippedEvent } }> = []
  const batchEvents = new Map<number, Array<ExplainabilityEnvelope & { record: { event: GlobalMapBatchBuiltEvent } }>>()
  const pointsEvents: Array<ExplainabilityEnvelope & { record: { event: GlobalMapPointsProducedEvent } }> = []
  const llmStartedBySpan = new Map<string, Array<ExplainabilityEnvelope & { record: { event: LlmRequestStartedEvent } }>>()
  const llmCompletedBySpan = new Map<string, Array<ExplainabilityEnvelope & { record: { event: LlmRequestCompletedEvent } }>>()

  for (const envelope of ordered) {
    const event = envelope.record.event
    if (isGlobalContextBuilt(event)) contextBuilt = typedEnvelope(envelope, event)
    else if (isGlobalMapStarted(event)) mapStarted = typedEnvelope(envelope, event)
    else if (isGlobalMapBatchBuilt(event)) append(batchEvents, event.batch_index, typedEnvelope(envelope, event))
    else if (isGlobalMapPointsProduced(event)) pointsEvents.push(typedEnvelope(envelope, event))
    else if (isGlobalReduceContextBuilt(event)) reduceBuilt = typedEnvelope(envelope, event)
    else if (isGlobalReduceSkipped(event)) reduceSkippedEvents.push(typedEnvelope(envelope, event))
    else if (isLlmRequestStarted(event)) append(llmStartedBySpan, envelope.record.span_id, typedEnvelope(envelope, event))
    else if (isLlmRequestCompleted(event)) append(llmCompletedBySpan, envelope.record.span_id, typedEnvelope(envelope, event))
  }

  const claimedSequences = new Set<number>()
  const batches = new Map<number, BatchEvidence>()
  const mapSpan = mapStarted?.record.span_id
  for (const [batchIndex, events] of batchEvents) {
    if (mapSpan === undefined) continue
    const validEvents = events.filter((event) => event.record.parent_span_id === mapSpan)
    const latestBuilt = last(validEvents)
    if (latestBuilt === undefined) continue
    const canonicalBuiltEvents = validEvents.filter((event) => event.record.span_id === latestBuilt.record.span_id)
    const rawEvents: ExplainabilityEnvelope[] = [...canonicalBuiltEvents]
    canonicalBuiltEvents.forEach((event) => claimedSequences.add(event.sequence))
    const span = latestBuilt.record.span_id
    const lifecycle = selectLlmLifecycle(
      llmStartedBySpan.get(span) ?? [],
      llmCompletedBySpan.get(span) ?? [],
      mapSpan,
      latestBuilt.sequence,
    )
    const { started, completed } = lifecycle
    if (started !== undefined) rawEvents.push(started)
    if (completed !== undefined) rawEvents.push(completed)
    if (started !== undefined) claimedSequences.add(started.sequence)
    if (completed !== undefined) claimedSequences.add(completed.sequence)
    batches.set(batchIndex, { latestBuilt, rawEvents, started, completed })
  }

  for (const produced of pointsEvents) {
    const batch = batches.get(produced.record.event.batch_index)
    if (batch === undefined
      || produced.record.span_id !== batch.latestBuilt.record.span_id
      || produced.record.parent_span_id !== mapSpan
      || batch.completed === undefined
      || produced.sequence < batch.completed.sequence) continue
    const completionSequence = batch.completed.sequence
    const laterStart = (llmStartedBySpan.get(produced.record.span_id) ?? []).some((event) => event.record.parent_span_id === mapSpan
      && event.sequence > completionSequence
      && event.sequence <= produced.sequence)
    if (laterStart) continue
    if (batch.produced === undefined || produced.sequence >= batch.produced.sequence) batch.produced = produced
  }
  for (const batch of batches.values()) {
    if (batch.produced === undefined) continue
    batch.rawEvents.push(batch.produced)
    claimedSequences.add(batch.produced.sequence)
  }

  const batchViews = [...batches.values()]
    .map(buildBatchView)
    .sort((left, right) => left.batchIndex - right.batchIndex)
  const communityRaw = ordered.filter((envelope) => isGlobalContextBuilt(envelope.record.event)
    || isGlobalMapStarted(envelope.record.event)
    || (isGlobalMapBatchBuilt(envelope.record.event) && claimedSequences.has(envelope.sequence)))
  communityRaw.forEach((event) => claimedSequences.add(event.sequence))
  const mapRaw = [...batches.values()].flatMap((batch) => batch.rawEvents).sort(bySequence)
  mapRaw.forEach((event) => claimedSequences.add(event.sequence))

  const compatibleReduceSkipped = reduceBuilt === undefined || reduceBuilt.record.event.positive_point_count > 0
    ? undefined
    : last(reduceSkippedEvents.filter((envelope) => envelope.record.span_id === reduceBuilt.record.span_id
      && envelope.record.parent_span_id === reduceBuilt.record.parent_span_id
      && envelope.sequence > reduceBuilt.sequence))
  const reduceRaw: ExplainabilityEnvelope[] = []
  if (reduceBuilt !== undefined) reduceRaw.push(reduceBuilt)
  if (compatibleReduceSkipped !== undefined) reduceRaw.push(compatibleReduceSkipped)
  reduceRaw.sort(bySequence)
  reduceRaw.forEach((event) => claimedSequences.add(event.sequence))
  const reduceSpan = reduceBuilt?.record.span_id
  const reduceParentSpan = reduceBuilt?.record.parent_span_id
  const reduceLifecycle = reduceSpan === undefined || reduceParentSpan === undefined || reduceBuilt === undefined || compatibleReduceSkipped !== undefined
    ? {}
    : selectLlmLifecycle(
      llmStartedBySpan.get(reduceSpan) ?? [],
      llmCompletedBySpan.get(reduceSpan) ?? [],
      reduceParentSpan,
      reduceBuilt.sequence,
    )
  const answerRaw: ExplainabilityEnvelope[] = []
  if (reduceLifecycle.started !== undefined) answerRaw.push(reduceLifecycle.started)
  if (reduceLifecycle.completed !== undefined) answerRaw.push(reduceLifecycle.completed)
  answerRaw.sort(bySequence)
  answerRaw.forEach((event) => claimedSequences.add(event.sequence))

  const reportCount = contextBuilt?.record.event.report_count
  const communityBatchCount = contextBuilt?.record.event.batch_count
  const mapBatchCount = mapStarted?.record.event.batch_count
  const decisions = reduceBuilt?.record.event.points.map((point) => ({
    ...point,
    identity: pointIdentity(point.batch_index, point.point_index),
  })) ?? []
  const nonPositiveCount = decisions.filter((point) => point.reason === "non_positive_score").length
  const tokenBudgetExcludedCount = decisions.filter((point) => point.reason === "token_budget").length
  const reduceSummary: GlobalReduceSummary = {
    built: reduceBuilt !== undefined,
    candidatePointCount: reduceBuilt?.record.event.candidate_point_count,
    positivePointCount: reduceBuilt?.record.event.positive_point_count,
    selectedPointCount: reduceBuilt?.record.event.selected_point_count,
    nonPositiveCount: reduceBuilt === undefined ? undefined : nonPositiveCount,
    tokenBudgetExcludedCount: reduceBuilt === undefined ? undefined : tokenBudgetExcludedCount,
    tokenBudget: reduceBuilt?.record.event.token_budget,
    tokensUsed: reduceBuilt?.record.event.tokens_used,
    truncated: reduceBuilt?.record.event.truncated ?? false,
    decisions,
    exactContext: reduceBuilt?.record.event.context ?? null,
    skippedReason: compatibleReduceSkipped?.record.event.reason ?? null,
  }
  const reduceStarted = reduceLifecycle.started
  const reduceCompleted = reduceLifecycle.completed
  const noDataAnswer = compatibleReduceSkipped?.record.event.reason === "no_positive_points"
  const answerSummary: GlobalAnswerGenerationSummary = {
    calls: noDataAnswer ? 0 : Number(reduceStarted !== undefined || reduceCompleted !== undefined),
    generated: reduceCompleted !== undefined,
    noDataAnswer,
    model: reduceCompleted?.record.event.model_id ?? reduceStarted?.record.event.model_id,
    inputTokens: reduceCompleted?.record.event.input_tokens ?? reduceStarted?.record.event.prompt_tokens,
    outputTokens: reduceCompleted?.record.event.output_tokens,
    elapsedMs: reduceCompleted?.record.event.elapsed_ms,
    exactPrompt: reduceStarted?.record.event.prompt ?? null,
    rawResponse: reduceCompleted?.record.event.response ?? null,
  }

  const steps: GlobalSemanticStep[] = [
    {
      id: "community-context",
      kind: "community-context",
      title: "Community Context",
      rawEvents: communityRaw,
      focusEnvelope: null,
      summary: {
        built: contextBuilt !== undefined,
        reportCount,
        batchCount: communityBatchCount,
        tokensUsed: batchViews.reduce((total, batch) => total + batch.tokensUsed, 0),
        batches: batchViews,
      },
    },
    {
      id: "map-analysis",
      kind: "map-analysis",
      title: "Map Analysis",
      rawEvents: mapRaw,
      focusEnvelope: null,
      summary: {
        started: mapStarted !== undefined,
        batchCount: mapBatchCount,
        analystCalls: batchViews.filter((batch) => batch.model !== undefined).length,
        pointCount: batchViews.reduce((total, batch) => total + batch.points.length, 0),
        positivePointCount: batchViews.reduce(
          (total, batch) => total + batch.points.filter((point) => point.score > 0).length,
          0,
        ),
        batches: batchViews,
      },
    },
    {
      id: "evidence-reduction",
      kind: "evidence-reduction",
      title: "Evidence Reduction",
      rawEvents: reduceRaw,
      focusEnvelope: null,
      summary: reduceSummary,
    },
    {
      id: "global-answer-generation",
      kind: "global-answer-generation",
      title: "Answer Generation",
      rawEvents: answerRaw,
      focusEnvelope: null,
      summary: answerSummary,
    },
  ]
  const diagnosticEvents = ordered.filter((envelope) => !claimedSequences.has(envelope.sequence))
  return { steps, diagnosticEvents }
}

function buildBatchView(batch: BatchEvidence): GlobalMapBatchView {
  const event = batch.latestBuilt.record.event
  const points = batch.produced?.record.event.points
    .map((point) => ({ ...point, identity: pointIdentity(point.batch_index, point.point_index) }))
    .sort((left, right) => left.point_index - right.point_index) ?? []
  const status: GlobalMapBatchStatus = batch.produced !== undefined
    ? "completed"
    : batch.completed !== undefined
      ? "response_received"
      : batch.started !== undefined
        ? "analyzing"
        : "ready"
  return {
    batchIndex: event.batch_index,
    spanId: batch.latestBuilt.record.span_id,
    reportCount: event.report_count,
    reportIds: [...event.report_ids],
    tokensUsed: event.tokens_used,
    tokenBudget: event.token_budget,
    exactContext: event.context ?? null,
    status,
    model: batch.completed?.record.event.model_id ?? batch.started?.record.event.model_id,
    inputTokens: batch.completed?.record.event.input_tokens ?? batch.started?.record.event.prompt_tokens,
    outputTokens: batch.completed?.record.event.output_tokens,
    elapsedMs: batch.completed?.record.event.elapsed_ms,
    exactPrompt: batch.started?.record.event.prompt ?? null,
    rawResponse: batch.completed?.record.event.response ?? null,
    points,
    rawEvents: batch.rawEvents.sort(bySequence),
  }
}

function pointIdentity(batchIndex: number, pointIndex: number): string {
  return `${batchIndex}:${pointIndex}`
}

function append<K, V>(target: Map<K, V[]>, key: K, value: V): void {
  const values = target.get(key) ?? []
  values.push(value)
  target.set(key, values)
}

function last<T>(values: readonly T[]): T | undefined {
  return values.at(-1)
}

interface LlmLifecycle {
  started?: ExplainabilityEnvelope & { record: { event: LlmRequestStartedEvent } }
  completed?: ExplainabilityEnvelope & { record: { event: LlmRequestCompletedEvent } }
}

function selectLlmLifecycle(
  starts: readonly (ExplainabilityEnvelope & { record: { event: LlmRequestStartedEvent } })[],
  completions: readonly (ExplainabilityEnvelope & { record: { event: LlmRequestCompletedEvent } })[],
  parentSpan: string,
  afterSequence: number,
): LlmLifecycle {
  const validStarts = starts.filter((event) => event.record.parent_span_id === parentSpan && event.sequence > afterSequence)
  const validCompletions = completions.filter((event) => event.record.parent_span_id === parentSpan && event.sequence > afterSequence)
  for (let index = validCompletions.length - 1; index >= 0; index -= 1) {
    const completed = validCompletions[index]
    if (completed === undefined) continue
    const started = last(validStarts.filter((event) => event.sequence <= completed.sequence))
    if (started !== undefined) return { started, completed }
  }
  return { started: last(validStarts) }
}

function bySequence(left: ExplainabilityEnvelope, right: ExplainabilityEnvelope): number {
  return left.sequence - right.sequence
}

function typedEnvelope<T extends ExplainabilityEventPayload>(
  envelope: ExplainabilityEnvelope,
  event: T,
): ExplainabilityEnvelope & { record: { event: T } } {
  return { ...envelope, record: { ...envelope.record, event } }
}

function isGlobalContextBuilt(event: ExplainabilityEventPayload): event is GlobalContextBuiltEvent {
  return event.type === "global_context_built" && finiteNumber(event.batch_count) && finiteNumber(event.report_count)
}

function isGlobalMapStarted(event: ExplainabilityEventPayload): event is GlobalMapStartedEvent {
  return event.type === "global_map_started" && finiteNumber(event.batch_count)
}

function isGlobalMapBatchBuilt(event: ExplainabilityEventPayload): event is GlobalMapBatchBuiltEvent {
  return event.type === "global_map_batch_built"
    && finiteNumber(event.batch_index)
    && finiteNumber(event.report_count)
    && stringArray(event.report_ids)
    && finiteNumber(event.tokens_used)
    && finiteNumber(event.token_budget)
    && optionalString(event.context)
}

function isGlobalMapPointsProduced(event: ExplainabilityEventPayload): event is GlobalMapPointsProducedEvent {
  return event.type === "global_map_points_produced"
    && finiteNumber(event.batch_index)
    && Array.isArray(event.points)
    && event.points.every(isMapPoint)
}

function isGlobalReduceContextBuilt(event: ExplainabilityEventPayload): event is GlobalReduceContextBuiltEvent {
  return event.type === "global_reduce_context_built"
    && finiteNumber(event.candidate_point_count)
    && finiteNumber(event.positive_point_count)
    && finiteNumber(event.selected_point_count)
    && finiteNumber(event.token_budget)
    && finiteNumber(event.tokens_used)
    && typeof event.truncated === "boolean"
    && Array.isArray(event.points)
    && event.points.every(isMapPointDecision)
    && optionalString(event.context)
}

function isGlobalReduceSkipped(event: ExplainabilityEventPayload): event is GlobalReduceSkippedEvent {
  return event.type === "global_reduce_skipped" && event.reason === "no_positive_points"
}

function isLlmRequestStarted(event: ExplainabilityEventPayload): event is LlmRequestStartedEvent {
  return event.type === "llm_request_started"
    && typeof event.model_id === "string"
    && finiteNumber(event.prompt_tokens)
    && optionalString(event.prompt)
}

function isLlmRequestCompleted(event: ExplainabilityEventPayload): event is LlmRequestCompletedEvent {
  return event.type === "llm_request_completed"
    && typeof event.model_id === "string"
    && finiteNumber(event.input_tokens)
    && finiteNumber(event.output_tokens)
    && finiteNumber(event.elapsed_ms)
    && optionalString(event.response)
}

function isMapPoint(value: unknown): value is GlobalMapPointEvidence {
  if (!isObject(value)) return false
  return finiteNumber(value.batch_index)
    && finiteNumber(value.point_index)
    && finiteNumber(value.score)
    && optionalString(value.answer)
}

function isMapPointDecision(value: unknown): value is GlobalMapPointDecision {
  if (!isObject(value)) return false
  return finiteNumber(value.batch_index)
    && finiteNumber(value.point_index)
    && finiteNumber(value.score)
    && optionalString(value.answer)
    && typeof value.selected === "boolean"
    && (value.reason === "selected" || value.reason === "non_positive_score" || value.reason === "token_budget")
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value)
}

function optionalString(value: unknown): value is string | undefined {
  return value === undefined || typeof value === "string"
}

function stringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string")
}
