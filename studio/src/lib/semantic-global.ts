import type {
  DynamicCommunityRatingAttemptStartedEvent,
  DynamicCommunityRatingEvidence,
  DynamicCommunitySelectionCompletedEvent,
  DynamicCommunitySelectionStartedEvent,
  DynamicCommunityTraversalWaveStartedEvent,
  DynamicTraversalWaveSource,
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
export type DynamicRatingAttemptStatus = "rating" | "response_received"

export interface DynamicTraversalWaveView {
  waveIndex: number
  source: DynamicTraversalWaveSource
  communityIds: string[]
}

export interface DynamicRatingAttemptView {
  identity: string
  spanId: string
  communityId: string
  reportId: string
  repeatIndex: number
  repeatCount: number
  status: DynamicRatingAttemptStatus
  model?: string
  inputTokens?: number
  outputTokens?: number
  elapsedMs?: number
  exactPrompt: string | null
  rawResponse: string | null
  rawEvents: ExplainabilityEnvelope[]
}

export interface DynamicCommunityDecisionView extends DynamicCommunityRatingEvidence {
  attempts: DynamicRatingAttemptView[]
}

export interface DynamicCommunitySelectionSummary {
  started: boolean
  completed: boolean
  initialCommunityCount?: number
  threshold?: number
  maxLevel?: number
  keepParent?: boolean
  useSummary?: boolean
  numRepeats?: number
  activeWave?: number
  visitedCount?: number
  thresholdPassedCount?: number
  selectedCount?: number
  attemptsStarted: number
  attemptsCompleted: number
  waves: DynamicTraversalWaveView[]
  decisions: DynamicCommunityDecisionView[]
}

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
  noDataPathSelected: boolean
  noDataAnswerReturned: boolean
  model?: string
  inputTokens?: number
  outputTokens?: number
  elapsedMs?: number
  exactPrompt: string | null
  rawResponse: string | null
}

export type GlobalSemanticStep =
  | { id: "community-selection"; kind: "community-selection"; title: "Community Selection"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: DynamicCommunitySelectionSummary }
  | { id: "community-context"; kind: "community-context"; title: "Community Context"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalCommunityContextSummary }
  | { id: "map-analysis"; kind: "map-analysis"; title: "Map Analysis"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalMapAnalysisSummary }
  | { id: "evidence-reduction"; kind: "evidence-reduction"; title: "Evidence Reduction"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalReduceSummary }
  | { id: "global-answer-generation"; kind: "global-answer-generation"; title: "Answer Generation"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: GlobalAnswerGenerationSummary }

export interface GlobalSemanticTimelineModel {
  variant: "static" | "dynamic"
  steps: GlobalSemanticStep[]
  diagnosticEvents: ExplainabilityEnvelope[]
}

export function isGlobalSemanticStep(step: { kind: string }): step is GlobalSemanticStep {
  return step.kind === "community-selection"
    || step.kind === "community-context"
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
  const selectionStartedEvents: Array<ExplainabilityEnvelope & { record: { event: DynamicCommunitySelectionStartedEvent } }> = []
  const selectionWaveEvents: Array<ExplainabilityEnvelope & { record: { event: DynamicCommunityTraversalWaveStartedEvent } }> = []
  const ratingAttemptEvents: Array<ExplainabilityEnvelope & { record: { event: DynamicCommunityRatingAttemptStartedEvent } }> = []
  const selectionCompletedEvents: Array<ExplainabilityEnvelope & { record: { event: DynamicCommunitySelectionCompletedEvent } }> = []
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
    if (isDynamicSelectionStarted(event)) selectionStartedEvents.push(typedEnvelope(envelope, event))
    else if (isDynamicTraversalWaveStarted(event)) selectionWaveEvents.push(typedEnvelope(envelope, event))
    else if (isDynamicRatingAttemptStarted(event)) ratingAttemptEvents.push(typedEnvelope(envelope, event))
    else if (isDynamicSelectionCompleted(event)) selectionCompletedEvents.push(typedEnvelope(envelope, event))
    else if (isGlobalContextBuilt(event)) contextBuilt = typedEnvelope(envelope, event)
    else if (isGlobalMapStarted(event)) mapStarted = typedEnvelope(envelope, event)
    else if (isGlobalMapBatchBuilt(event)) append(batchEvents, event.batch_index, typedEnvelope(envelope, event))
    else if (isGlobalMapPointsProduced(event)) pointsEvents.push(typedEnvelope(envelope, event))
    else if (isGlobalReduceContextBuilt(event)) reduceBuilt = typedEnvelope(envelope, event)
    else if (isGlobalReduceSkipped(event)) reduceSkippedEvents.push(typedEnvelope(envelope, event))
    else if (isLlmRequestStarted(event)) append(llmStartedBySpan, envelope.record.span_id, typedEnvelope(envelope, event))
    else if (isLlmRequestCompleted(event)) append(llmCompletedBySpan, envelope.record.span_id, typedEnvelope(envelope, event))
  }

  const claimedSequences = new Set<number>()
  const queryRootSpan = ordered.find((envelope) => envelope.record.event.type === "query_started"
    && envelope.record.event.method === "global"
    && envelope.record.parent_span_id === undefined)?.record.span_id
  const dynamicSelection = buildDynamicSelection(
    queryRootSpan,
    selectionStartedEvents,
    selectionWaveEvents,
    ratingAttemptEvents,
    selectionCompletedEvents,
    llmStartedBySpan,
    llmCompletedBySpan,
  )
  dynamicSelection?.rawEvents.forEach((event) => claimedSequences.add(event.sequence))
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
  const noDataPathSelected = compatibleReduceSkipped?.record.event.reason === "no_positive_points"
  const terminal = last(ordered.filter((envelope) => (envelope.record.event.type === "run_completed"
    || envelope.record.event.type === "run_failed")
    && envelope.record.span_id === queryRootSpan
    && envelope.record.parent_span_id === undefined))
  const noDataAnswerReturned = noDataPathSelected
    && terminal?.record.event.type === "run_completed"
    && terminal.sequence > (compatibleReduceSkipped?.sequence ?? Number.MAX_SAFE_INTEGER)
  const answerSummary: GlobalAnswerGenerationSummary = {
    calls: noDataPathSelected ? 0 : Number(reduceStarted !== undefined || reduceCompleted !== undefined),
    generated: reduceCompleted !== undefined,
    noDataPathSelected,
    noDataAnswerReturned,
    model: reduceCompleted?.record.event.model_id ?? reduceStarted?.record.event.model_id,
    inputTokens: reduceCompleted?.record.event.input_tokens ?? reduceStarted?.record.event.prompt_tokens,
    outputTokens: reduceCompleted?.record.event.output_tokens,
    elapsedMs: reduceCompleted?.record.event.elapsed_ms,
    exactPrompt: reduceStarted?.record.event.prompt ?? null,
    rawResponse: reduceCompleted?.record.event.response ?? null,
  }

  const steps: GlobalSemanticStep[] = [
    ...(dynamicSelection === undefined ? [] : [{
      id: "community-selection" as const,
      kind: "community-selection" as const,
      title: "Community Selection" as const,
      rawEvents: dynamicSelection.rawEvents,
      focusEnvelope: null,
      summary: dynamicSelection.summary,
    }]),
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
  return { variant: dynamicSelection === undefined ? "static" : "dynamic", steps, diagnosticEvents }
}

interface DynamicSelectionBuild {
  rawEvents: ExplainabilityEnvelope[]
  summary: DynamicCommunitySelectionSummary
}

function buildDynamicSelection(
  queryRootSpan: string | undefined,
  starts: readonly (ExplainabilityEnvelope & { record: { event: DynamicCommunitySelectionStartedEvent } })[],
  waves: readonly (ExplainabilityEnvelope & { record: { event: DynamicCommunityTraversalWaveStartedEvent } })[],
  attempts: readonly (ExplainabilityEnvelope & { record: { event: DynamicCommunityRatingAttemptStartedEvent } })[],
  completions: readonly (ExplainabilityEnvelope & { record: { event: DynamicCommunitySelectionCompletedEvent } })[],
  llmStartedBySpan: ReadonlyMap<string, Array<ExplainabilityEnvelope & { record: { event: LlmRequestStartedEvent } }>>,
  llmCompletedBySpan: ReadonlyMap<string, Array<ExplainabilityEnvelope & { record: { event: LlmRequestCompletedEvent } }>>,
): DynamicSelectionBuild | undefined {
  if (queryRootSpan === undefined) return undefined
  const rootStarts = starts.filter((event) => event.record.parent_span_id === queryRootSpan)
  const rootWaves = waves.filter((event) => event.record.parent_span_id === queryRootSpan)
  const rootSelectionSpans = new Set([...rootStarts, ...rootWaves].map((event) => event.record.span_id))
  const rootAttempts = attempts.filter((event) => rootSelectionSpans.has(event.record.parent_span_id ?? ""))
  const completed = [...completions].reverse().find((candidate) => {
    if (candidate.record.parent_span_id !== queryRootSpan) return false
    const candidateStart = last(rootStarts.filter((event) => event.record.span_id === candidate.record.span_id
      && event.record.parent_span_id === candidate.record.parent_span_id
      && event.sequence < candidate.sequence))
    return candidateStart !== undefined && candidate.record.event.ratings.every((rating) =>
      rating.threshold_passed === (rating.selected_rating >= candidateStart.record.event.threshold))
  })
  const latestSignal = last([...rootStarts, ...rootWaves, ...rootAttempts].sort(bySequence))
  const selectionSpan = completed?.record.span_id
    ?? (latestSignal?.record.event.type === "dynamic_community_rating_attempt_started"
      ? latestSignal.record.parent_span_id
      : latestSignal?.record.span_id)
  if (selectionSpan === undefined) return undefined

  const selectionParent = completed?.record.parent_span_id
    ?? last([...rootStarts, ...rootWaves]
      .filter((event) => event.record.span_id === selectionSpan)
      .sort(bySequence))?.record.parent_span_id
  const upperBound = completed?.sequence ?? Number.POSITIVE_INFINITY
  const started = last(rootStarts.filter((event) => event.record.span_id === selectionSpan
    && event.record.parent_span_id === selectionParent
    && event.sequence < upperBound))
  const lowerBound = started?.sequence ?? Number.NEGATIVE_INFINITY
  const matchingWaves = rootWaves.filter((event) => event.record.span_id === selectionSpan
    && event.record.parent_span_id === selectionParent
    && event.sequence > lowerBound
    && event.sequence < upperBound)
  const latestWaveByIndex = new Map<number, ExplainabilityEnvelope & { record: { event: DynamicCommunityTraversalWaveStartedEvent } }>()
  for (const wave of matchingWaves) latestWaveByIndex.set(wave.record.event.wave_index, wave)
  const canonicalWaves = [...latestWaveByIndex.values()].sort(bySequence)
  const canonicalCommunityIds = new Set(canonicalWaves.flatMap((wave) => wave.record.event.community_ids))
  const finalReports = new Map(completed?.record.event.ratings.map((rating) => [rating.community_id, rating.report_id]) ?? [])
  const latestAttemptByIdentity = new Map<string, ExplainabilityEnvelope & { record: { event: DynamicCommunityRatingAttemptStartedEvent } }>()
  for (const attempt of rootAttempts) {
    if (attempt.record.parent_span_id !== selectionSpan
      || attempt.sequence <= lowerBound
      || attempt.sequence >= upperBound) continue
    if (canonicalWaves.length > 0 && !canonicalCommunityIds.has(attempt.record.event.community_id)) continue
    if (started !== undefined && attempt.record.event.repeat_count !== started.record.event.num_repeats) continue
    const finalReportId = finalReports.get(attempt.record.event.community_id)
    if (completed !== undefined
      && (finalReportId === undefined || finalReportId !== attempt.record.event.report_id)) continue
    const identity = dynamicAttemptIdentity(attempt.record.event.community_id, attempt.record.event.repeat_index)
    latestAttemptByIdentity.set(identity, attempt)
  }

  const attemptViews = [...latestAttemptByIdentity.values()]
    .map((attempt) => buildDynamicAttemptView(
      attempt,
      selectionSpan,
      upperBound,
      llmStartedBySpan.get(attempt.record.span_id) ?? [],
      llmCompletedBySpan.get(attempt.record.span_id) ?? [],
    ))
    .sort((left, right) => left.communityId.localeCompare(right.communityId) || left.repeatIndex - right.repeatIndex)
  const attemptsByCommunity = new Map<string, DynamicRatingAttemptView[]>()
  for (const attempt of attemptViews) append(attemptsByCommunity, attempt.communityId, attempt)
  const decisions = completed?.record.event.ratings.map((rating) => ({
    ...rating,
    attempts: [...(attemptsByCommunity.get(rating.community_id) ?? [])]
      .sort((left, right) => left.repeatIndex - right.repeatIndex),
  })) ?? []
  const rawEvents = [
    ...(started === undefined ? [] : [started]),
    ...canonicalWaves,
    ...attemptViews.flatMap((attempt) => attempt.rawEvents),
    ...(completed === undefined ? [] : [completed]),
  ].sort(bySequence)
  const latestWave = last(canonicalWaves)
  return {
    rawEvents,
    summary: {
      started: started !== undefined,
      completed: completed !== undefined,
      initialCommunityCount: started?.record.event.initial_community_count,
      threshold: started?.record.event.threshold,
      maxLevel: started?.record.event.max_level,
      keepParent: started?.record.event.keep_parent,
      useSummary: started?.record.event.use_summary,
      numRepeats: started?.record.event.num_repeats,
      activeWave: completed === undefined ? latestWave?.record.event.wave_index : undefined,
      visitedCount: completed?.record.event.visited_count,
      thresholdPassedCount: completed?.record.event.threshold_passed_count,
      selectedCount: completed?.record.event.selected_count,
      attemptsStarted: attemptViews.length,
      attemptsCompleted: attemptViews.filter((attempt) => attempt.status === "response_received").length,
      waves: canonicalWaves
        .map((wave) => ({
          waveIndex: wave.record.event.wave_index,
          source: wave.record.event.source,
          communityIds: [...wave.record.event.community_ids],
        }))
        .sort((left, right) => left.waveIndex - right.waveIndex),
      decisions,
    },
  }
}

function buildDynamicAttemptView(
  attempt: ExplainabilityEnvelope & { record: { event: DynamicCommunityRatingAttemptStartedEvent } },
  selectionSpan: string,
  upperBound: number,
  starts: readonly (ExplainabilityEnvelope & { record: { event: LlmRequestStartedEvent } })[],
  completions: readonly (ExplainabilityEnvelope & { record: { event: LlmRequestCompletedEvent } })[],
): DynamicRatingAttemptView {
  const lifecycle = selectLlmLifecycle(
    starts.filter((event) => event.sequence < upperBound),
    completions.filter((event) => event.sequence < upperBound),
    selectionSpan,
    attempt.sequence,
  )
  const rawEvents = [
    attempt,
    ...(lifecycle.started === undefined ? [] : [lifecycle.started]),
    ...(lifecycle.completed === undefined ? [] : [lifecycle.completed]),
  ].sort(bySequence)
  return {
    identity: dynamicAttemptIdentity(attempt.record.event.community_id, attempt.record.event.repeat_index),
    spanId: attempt.record.span_id,
    communityId: attempt.record.event.community_id,
    reportId: attempt.record.event.report_id,
    repeatIndex: attempt.record.event.repeat_index,
    repeatCount: attempt.record.event.repeat_count,
    status: lifecycle.completed === undefined ? "rating" : "response_received",
    model: lifecycle.completed?.record.event.model_id ?? lifecycle.started?.record.event.model_id,
    inputTokens: lifecycle.completed?.record.event.input_tokens ?? lifecycle.started?.record.event.prompt_tokens,
    outputTokens: lifecycle.completed?.record.event.output_tokens,
    elapsedMs: lifecycle.completed?.record.event.elapsed_ms,
    exactPrompt: lifecycle.started?.record.event.prompt ?? null,
    rawResponse: lifecycle.completed?.record.event.response ?? null,
    rawEvents,
  }
}

function dynamicAttemptIdentity(communityId: string, repeatIndex: number): string {
  return `${communityId}:${repeatIndex}`
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

function isDynamicSelectionStarted(event: ExplainabilityEventPayload): event is DynamicCommunitySelectionStartedEvent {
  return event.type === "dynamic_community_selection_started"
    && positiveInteger(event.initial_community_count)
    && safeInteger(event.threshold)
    && nonNegativeInteger(event.max_level)
    && typeof event.keep_parent === "boolean"
    && typeof event.use_summary === "boolean"
    && positiveInteger(event.num_repeats)
}

function isDynamicTraversalWaveStarted(event: ExplainabilityEventPayload): event is DynamicCommunityTraversalWaveStartedEvent {
  return event.type === "dynamic_community_traversal_wave_started"
    && nonNegativeInteger(event.wave_index)
    && (event.source === "initial" || event.source === "child_expansion" || event.source === "fallback")
    && nonEmptyStringArray(event.community_ids)
}

function isDynamicRatingAttemptStarted(event: ExplainabilityEventPayload): event is DynamicCommunityRatingAttemptStartedEvent {
  return event.type === "dynamic_community_rating_attempt_started"
    && nonEmptyString(event.community_id)
    && nonEmptyString(event.report_id)
    && nonNegativeInteger(event.repeat_index)
    && positiveInteger(event.repeat_count)
    && event.repeat_index < event.repeat_count
}

function isDynamicSelectionCompleted(event: ExplainabilityEventPayload): event is DynamicCommunitySelectionCompletedEvent {
  const selectedCommunityIds = event.selected_community_ids
  const selectedReportIds = event.selected_report_ids
  const ratings = event.ratings
  if (event.type !== "dynamic_community_selection_completed"
    || !nonNegativeInteger(event.visited_count)
    || !nonNegativeInteger(event.threshold_passed_count)
    || !nonNegativeInteger(event.selected_count)
    || !stringArray(selectedCommunityIds)
    || !stringArray(selectedReportIds)
    || !selectedCommunityIds.every(nonEmptyString)
    || !selectedReportIds.every(nonEmptyString)
    || !Array.isArray(ratings)
    || !ratings.every(isDynamicRatingEvidence)) return false
  const passed = ratings.filter((rating) => rating.threshold_passed)
  const selected = ratings.filter((rating) => rating.selected)
  return event.visited_count === ratings.length
    && event.threshold_passed_count === passed.length
    && event.selected_count === selected.length
    && selectedCommunityIds.length === selected.length
    && selectedReportIds.length === selected.length
    && selected.every((rating, index) => rating.threshold_passed
      && rating.community_id === selectedCommunityIds[index]
      && rating.report_id === selectedReportIds[index])
}

function isDynamicRatingEvidence(value: unknown): value is DynamicCommunityRatingEvidence {
  if (!isObject(value)) return false
  return nonEmptyString(value.community_id)
    && nonEmptyString(value.report_id)
    && safeInteger(value.level)
    && safeInteger(value.selected_rating)
    && typeof value.threshold_passed === "boolean"
    && typeof value.selected === "boolean"
    && (!value.selected || value.threshold_passed)
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

function safeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value)
}

function nonNegativeInteger(value: unknown): value is number {
  return safeInteger(value) && value >= 0
}

function positiveInteger(value: unknown): value is number {
  return safeInteger(value) && value > 0
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0
}

function nonEmptyStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.length > 0 && value.every(nonEmptyString)
}

function optionalString(value: unknown): value is string | undefined {
  return value === undefined || typeof value === "string"
}

function stringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string")
}
