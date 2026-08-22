import type {
  DriftActionAttemptCompletedEvent,
  DriftActionAttemptStartedEvent,
  DriftActionContextBuiltEvent,
  DriftDepthActionsSelectedEvent,
  DriftExplorationStartedEvent,
  DriftHydeCompletedEvent,
  DriftHydeStartedEvent,
  DriftPrimerCompletedEvent,
  DriftPrimerFoldCompletedEvent,
  DriftPrimerFoldStartedEvent,
  DriftPrimerStartedEvent,
  DriftRankedReportEvidence,
  DriftReduceContextBuiltEvent,
  DriftReportsRankedEvent,
  EmbeddingCompletedEvent,
  EmbeddingStartedEvent,
  ExplainabilityEnvelope,
  ExplainabilityEventPayload,
  LlmRequestCompletedEvent,
  LlmRequestStartedEvent,
} from "@/api/types"

export type DriftPrimerStatus = "hyde" | "embedding" | "ranking" | "primer" | "ready"
export type DriftAttemptStatus = "running" | "incomplete" | "completed_empty" | "completed"
export type DriftActionStatus = "root" | "not_explored" | DriftAttemptStatus

export interface DriftLlmView {
  model?: string
  inputTokens?: number
  outputTokens?: number
  elapsedMs?: number
  exactPrompt: string | null
  rawResponse: string | null
}

export interface DriftHydeView extends DriftLlmView {
  templateReportId: string
  templateShortId: string
  templateCommunityId: string
  templateIndex: number
  reportCount: number
  completed: boolean
  usedOriginalQuery?: boolean
  effectiveQuery: string | null
}

export interface DriftPrimerFoldView extends DriftLlmView {
  foldIndex: number
  foldCount: number
  reportIds: string[]
  completed: boolean
  score?: number
  followUpCount?: number
  intermediateAnswer: string | null
  followUpQueries: string[] | null
  rawEvents: ExplainabilityEnvelope[]
}

export interface DriftPrimerSummary {
  status: DriftPrimerStatus
  hyde: DriftHydeView | null
  embedding: {
    model?: string
    promptTokens?: number
    dimensions?: number
    effectiveQuery: string | null
    completed: boolean
  } | null
  rankedReports: DriftRankedReportEvidence[]
  rankedReportCount?: number
  foldCount?: number
  folds: DriftPrimerFoldView[]
  aggregate: {
    score: number
    rootActionId: number
    followUpCount: number
    followUpActionIds: number[]
    answer: string | null
    followUpQueries: string[] | null
  } | null
}

export interface DriftDepthView {
  depthIndex: number
  candidateActionIds: number[]
  selectedActionIds: number[]
  selectionLimit: number
}

export interface DriftActionAttemptView extends DriftLlmView {
  identity: string
  spanId: string
  depthIndex: number
  actionId: number
  query: string | null
  context: string | null
  status: DriftAttemptStatus
  score?: number
  answerPresent?: boolean
  answerNonEmpty?: boolean
  followUpCount?: number
  targetActionIds: number[]
  answer: string | null
  followUpQueries: string[] | null
  rawEvents: ExplainabilityEnvelope[]
}

export interface DriftEdgeView {
  sourceActionId: number
  targetActionId: number
  ordinal: number
}

export interface DriftActionNodeView {
  actionId: number
  query: string | null
  status: DriftActionStatus
  attempts: DriftActionAttemptView[]
  outgoingEdges: DriftEdgeView[]
}

export interface DriftExplorationSummary {
  started: boolean
  maxDepth?: number
  selectionLimit?: number
  rootActionId?: number
  activeDepth?: number
  depths: DriftDepthView[]
  attempts: DriftActionAttemptView[]
  nodes: DriftActionNodeView[]
  edges: DriftEdgeView[]
  nodeCount?: number
  edgeCount?: number
}

export interface DriftSynthesisSummary extends DriftLlmView {
  built: boolean
  generating: boolean
  generated: boolean
  nodeCount?: number
  edgeCount?: number
  includedAnswerCount?: number
  includedActionIds: number[]
  exactStateContext: string | null
  exactReduceContext: string | null
}

export type DriftSemanticStep =
  | { id: "drift-primer-ranking"; kind: "drift-primer-ranking"; title: "Primer & Ranking"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: DriftPrimerSummary }
  | { id: "drift-exploration"; kind: "drift-exploration"; title: "Exploration"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: DriftExplorationSummary }
  | { id: "drift-final-synthesis"; kind: "drift-final-synthesis"; title: "Final Synthesis"; rawEvents: ExplainabilityEnvelope[]; focusEnvelope: null; summary: DriftSynthesisSummary }

export interface DriftSemanticTimelineModel {
  steps: DriftSemanticStep[]
  diagnosticEvents: ExplainabilityEnvelope[]
}

interface TypedEnvelope<T extends ExplainabilityEventPayload> extends ExplainabilityEnvelope {
  record: ExplainabilityEnvelope["record"] & { event: T }
}

export function isDriftSemanticStep(step: { kind: string }): step is DriftSemanticStep {
  return step.kind === "drift-primer-ranking"
    || step.kind === "drift-exploration"
    || step.kind === "drift-final-synthesis"
}

export function buildDriftSemanticTimeline(envelopes: readonly ExplainabilityEnvelope[]): DriftSemanticTimelineModel {
  const ordered = [...envelopes].sort(bySequence)
  const root = ordered.find((envelope) => envelope.record.parent_span_id === undefined
    && envelope.record.event.type === "query_started"
    && envelope.record.event.method === "drift")
  if (root === undefined) return { steps: [], diagnosticEvents: ordered }
  const rootSpan = root.record.span_id
  const terminal = ordered.find((envelope) => envelope.sequence > root.sequence
    && envelope.record.span_id === rootSpan
    && envelope.record.parent_span_id === undefined
    && (envelope.record.event.type === "run_completed" || envelope.record.event.type === "run_failed"))
  const cutoff = terminal?.sequence ?? Number.MAX_SAFE_INTEGER
  const claimed = new Set<number>()

  let hydeStarted: TypedEnvelope<DriftHydeStartedEvent> | undefined
  let hydeCompleted: TypedEnvelope<DriftHydeCompletedEvent> | undefined
  let embeddingStarted: TypedEnvelope<EmbeddingStartedEvent> | undefined
  let embeddingCompleted: TypedEnvelope<EmbeddingCompletedEvent> | undefined
  let ranked: TypedEnvelope<DriftReportsRankedEvent> | undefined
  let primerStarted: TypedEnvelope<DriftPrimerStartedEvent> | undefined
  let primerCompleted: TypedEnvelope<DriftPrimerCompletedEvent> | undefined
  let explorationStarted: TypedEnvelope<DriftExplorationStartedEvent> | undefined
  let reduceBuilt: TypedEnvelope<DriftReduceContextBuiltEvent> | undefined
  const foldStarts = new Map<number, TypedEnvelope<DriftPrimerFoldStartedEvent>>()
  const attemptStarts = new Map<string, TypedEnvelope<DriftActionAttemptStartedEvent>>()
  const depthSelections = new Map<number, TypedEnvelope<DriftDepthActionsSelectedEvent>>()
  const contextsBySpan = new Map<string, TypedEnvelope<DriftActionContextBuiltEvent>>()
  const foldCompletedBySpan = new Map<string, TypedEnvelope<DriftPrimerFoldCompletedEvent>>()
  const attemptCompletedBySpan = new Map<string, TypedEnvelope<DriftActionAttemptCompletedEvent>>()
  const llmStartedBySpan = new Map<string, TypedEnvelope<LlmRequestStartedEvent>>()
  const llmCompletedBySpan = new Map<string, TypedEnvelope<LlmRequestCompletedEvent>>()

  for (const envelope of ordered) {
    if (envelope.sequence <= root.sequence || envelope.sequence >= cutoff) continue
    const event = envelope.record.event
    const parent = envelope.record.parent_span_id
    const span = envelope.record.span_id
    if (isDriftHydeStarted(event) && parent === rootSpan && hydeStarted === undefined) hydeStarted = typed(envelope, event)
    else if (isDriftHydeCompleted(event) && hydeStarted?.record.span_id === span && parent === rootSpan && hydeCompleted === undefined) hydeCompleted = typed(envelope, event)
    else if (isEmbeddingStarted(event) && parent === rootSpan && hydeStarted !== undefined && embeddingStarted === undefined) embeddingStarted = typed(envelope, event)
    else if (isEmbeddingCompleted(event) && embeddingStarted?.record.span_id === span && parent === rootSpan && embeddingCompleted === undefined) embeddingCompleted = typed(envelope, event)
    else if (isDriftReportsRanked(event) && parent === rootSpan && ranked === undefined) ranked = typed(envelope, event)
    else if (isDriftPrimerStarted(event) && parent === rootSpan && primerStarted === undefined) primerStarted = typed(envelope, event)
    else if (isDriftPrimerFoldStarted(event)
      && primerStarted?.record.span_id === parent
      && !foldStarts.has(event.fold_index)
      && ![...foldStarts.values()].some((item) => item.record.span_id === span)) foldStarts.set(event.fold_index, typed(envelope, event))
    else if (isDriftPrimerFoldCompleted(event) && !foldCompletedBySpan.has(span)) foldCompletedBySpan.set(span, typed(envelope, event))
    else if (isDriftPrimerCompleted(event) && primerStarted?.record.span_id === span && parent === rootSpan && primerCompleted === undefined) primerCompleted = typed(envelope, event)
    else if (isDriftExplorationStarted(event) && parent === rootSpan && explorationStarted === undefined) explorationStarted = typed(envelope, event)
    else if (isDriftDepthActionsSelected(event)
      && explorationStarted?.record.span_id === span
      && parent === rootSpan
      && !depthSelections.has(event.depth_index)) depthSelections.set(event.depth_index, typed(envelope, event))
    else if (isDriftActionAttemptStarted(event)
      && explorationStarted?.record.span_id === parent
      && !attemptStarts.has(span)) attemptStarts.set(span, typed(envelope, event))
    else if (isDriftActionContextBuilt(event) && !contextsBySpan.has(span)) contextsBySpan.set(span, typed(envelope, event))
    else if (isDriftActionAttemptCompleted(event) && !attemptCompletedBySpan.has(span)) attemptCompletedBySpan.set(span, typed(envelope, event))
    else if (isDriftReduceContextBuilt(event) && parent === rootSpan && reduceBuilt === undefined) reduceBuilt = typed(envelope, event)
    else if (isLlmRequestStarted(event) && !llmStartedBySpan.has(span)) llmStartedBySpan.set(span, typed(envelope, event))
    else if (isLlmRequestCompleted(event) && !llmCompletedBySpan.has(span)) llmCompletedBySpan.set(span, typed(envelope, event))
  }

  const primerRaw: ExplainabilityEnvelope[] = []
  const hydeLlm = stageLlm(hydeStarted, rootSpan, llmStartedBySpan, llmCompletedBySpan, claimed, primerRaw)
  claim(hydeStarted, claimed, primerRaw)
  claim(hydeCompleted, claimed, primerRaw)
  claim(embeddingStarted, claimed, primerRaw)
  claim(embeddingCompleted, claimed, primerRaw)
  claim(ranked, claimed, primerRaw)
  claim(primerStarted, claimed, primerRaw)
  const foldViews = [...foldStarts.values()]
    .sort((left, right) => left.record.event.fold_index - right.record.event.fold_index)
    .map((started) => buildFold(started, primerStarted, foldCompletedBySpan, llmStartedBySpan, llmCompletedBySpan, claimed, primerRaw))
  claim(primerCompleted, claimed, primerRaw)

  const explorationRaw: ExplainabilityEnvelope[] = []
  claim(explorationStarted, claimed, explorationRaw)
  const depths = [...depthSelections.values()]
    .sort((left, right) => left.record.event.depth_index - right.record.event.depth_index)
    .map((envelope) => {
      claim(envelope, claimed, explorationRaw)
      const event = envelope.record.event
      return { depthIndex: event.depth_index, candidateActionIds: event.candidate_action_ids, selectedActionIds: event.selected_action_ids, selectionLimit: event.selection_limit }
    })
  const attempts = [...attemptStarts.values()]
    .sort(bySequence)
    .map((started) => buildAttempt(started, explorationStarted, contextsBySpan, attemptCompletedBySpan, llmStartedBySpan, llmCompletedBySpan, claimed, explorationRaw))
  const graph = buildActionGraph(root, primerCompleted, attempts)

  const synthesisRaw: ExplainabilityEnvelope[] = []
  claim(reduceBuilt, claimed, synthesisRaw)
  const reduceLlm = stageLlm(reduceBuilt, rootSpan, llmStartedBySpan, llmCompletedBySpan, claimed, synthesisRaw)

  const primerSummary: DriftPrimerSummary = {
    status: primerCompleted !== undefined
      ? "ready"
      : primerStarted !== undefined
        ? "primer"
        : ranked !== undefined
          ? "ranking"
          : embeddingStarted !== undefined
            ? "embedding"
            : "hyde",
    hyde: hydeStarted === undefined ? null : {
      ...hydeLlm,
      templateReportId: hydeStarted.record.event.template_report_id,
      templateShortId: hydeStarted.record.event.template_short_id,
      templateCommunityId: hydeStarted.record.event.template_community_id,
      templateIndex: hydeStarted.record.event.template_index,
      reportCount: hydeStarted.record.event.report_count,
      completed: hydeCompleted !== undefined,
      usedOriginalQuery: hydeCompleted?.record.event.used_original_query,
      effectiveQuery: embeddingStarted?.record.event.input ?? null,
    },
    embedding: embeddingStarted === undefined ? null : {
      model: embeddingCompleted?.record.event.model_id ?? embeddingStarted.record.event.model_id,
      promptTokens: embeddingCompleted?.record.event.prompt_tokens,
      dimensions: embeddingCompleted?.record.event.dimensions,
      effectiveQuery: embeddingStarted.record.event.input ?? null,
      completed: embeddingCompleted !== undefined,
    },
    rankedReports: ranked?.record.event.reports ?? [],
    rankedReportCount: primerStarted?.record.event.ranked_report_count ?? ranked?.record.event.reports.length,
    foldCount: primerStarted?.record.event.fold_count,
    folds: foldViews,
    aggregate: primerCompleted === undefined ? null : {
      score: primerCompleted.record.event.score,
      rootActionId: primerCompleted.record.event.root_action_id,
      followUpCount: primerCompleted.record.event.follow_up_count,
      followUpActionIds: primerCompleted.record.event.follow_up_action_ids,
      answer: primerCompleted.record.event.answer ?? null,
      followUpQueries: primerCompleted.record.event.follow_up_queries ?? null,
    },
  }
  const synthesisSummary: DriftSynthesisSummary = {
    built: reduceBuilt !== undefined,
    generating: reduceLlm.exactPrompt !== null && reduceLlm.rawResponse === null,
    generated: reduceLlm.completed !== undefined,
    nodeCount: reduceBuilt?.record.event.node_count,
    edgeCount: reduceBuilt?.record.event.edge_count,
    includedAnswerCount: reduceBuilt?.record.event.included_answer_count,
    includedActionIds: reduceBuilt?.record.event.included_action_ids ?? [],
    exactStateContext: reduceBuilt?.record.event.state_context ?? null,
    exactReduceContext: reduceBuilt?.record.event.reduce_context ?? null,
    ...reduceLlm,
  }
  return {
    steps: [
      { id: "drift-primer-ranking", kind: "drift-primer-ranking", title: "Primer & Ranking", rawEvents: primerRaw.sort(bySequence), focusEnvelope: null, summary: primerSummary },
      { id: "drift-exploration", kind: "drift-exploration", title: "Exploration", rawEvents: explorationRaw.sort(bySequence), focusEnvelope: null, summary: {
        started: explorationStarted !== undefined,
        maxDepth: explorationStarted?.record.event.max_depth,
        selectionLimit: explorationStarted?.record.event.selection_limit,
        rootActionId: explorationStarted?.record.event.root_action_id,
        activeDepth: depths.at(-1)?.depthIndex,
        depths,
        attempts,
        nodes: graph.nodes,
        edges: graph.edges,
        nodeCount: reduceBuilt?.record.event.node_count,
        edgeCount: reduceBuilt?.record.event.edge_count,
      } },
      { id: "drift-final-synthesis", kind: "drift-final-synthesis", title: "Final Synthesis", rawEvents: synthesisRaw.sort(bySequence), focusEnvelope: null, summary: synthesisSummary },
    ],
    diagnosticEvents: ordered.filter((envelope) => !claimed.has(envelope.sequence)),
  }
}

function buildFold(
  started: TypedEnvelope<DriftPrimerFoldStartedEvent>,
  primer: TypedEnvelope<DriftPrimerStartedEvent> | undefined,
  completedBySpan: ReadonlyMap<string, TypedEnvelope<DriftPrimerFoldCompletedEvent>>,
  llmStartedBySpan: ReadonlyMap<string, TypedEnvelope<LlmRequestStartedEvent>>,
  llmCompletedBySpan: ReadonlyMap<string, TypedEnvelope<LlmRequestCompletedEvent>>,
  claimed: Set<number>,
  raw: ExplainabilityEnvelope[],
): DriftPrimerFoldView {
  const span = started.record.span_id
  const parent = primer?.record.span_id
  claim(started, claimed, raw)
  const llm = stageLlm(started, parent, llmStartedBySpan, llmCompletedBySpan, claimed, raw)
  const candidate = completedBySpan.get(span)
  const completed = candidate !== undefined
    && candidate.record.parent_span_id === parent
    && candidate.record.event.fold_index === started.record.event.fold_index
    && candidate.sequence >= started.sequence ? candidate : undefined
  claim(completed, claimed, raw)
  return {
    ...llm,
    foldIndex: started.record.event.fold_index,
    foldCount: started.record.event.fold_count,
    reportIds: started.record.event.report_ids,
    completed: completed !== undefined,
    score: completed?.record.event.score,
    followUpCount: completed?.record.event.follow_up_count,
    intermediateAnswer: completed?.record.event.intermediate_answer ?? null,
    followUpQueries: completed?.record.event.follow_up_queries ?? null,
    rawEvents: [started, llm.started, llm.completed, completed].filter(defined).sort(bySequence),
  }
}

function buildAttempt(
  started: TypedEnvelope<DriftActionAttemptStartedEvent>,
  exploration: TypedEnvelope<DriftExplorationStartedEvent> | undefined,
  contextsBySpan: ReadonlyMap<string, TypedEnvelope<DriftActionContextBuiltEvent>>,
  completedBySpan: ReadonlyMap<string, TypedEnvelope<DriftActionAttemptCompletedEvent>>,
  llmStartedBySpan: ReadonlyMap<string, TypedEnvelope<LlmRequestStartedEvent>>,
  llmCompletedBySpan: ReadonlyMap<string, TypedEnvelope<LlmRequestCompletedEvent>>,
  claimed: Set<number>,
  raw: ExplainabilityEnvelope[],
): DriftActionAttemptView {
  const span = started.record.span_id
  const parent = exploration?.record.span_id
  claim(started, claimed, raw)
  const contextCandidate = contextsBySpan.get(span)
  const context = contextCandidate !== undefined
    && contextCandidate.record.parent_span_id === parent
    && contextCandidate.record.event.action_id === started.record.event.action_id
    && contextCandidate.sequence >= started.sequence ? contextCandidate : undefined
  claim(context, claimed, raw)
  const llm = stageLlm(started, parent, llmStartedBySpan, llmCompletedBySpan, claimed, raw)
  const completedCandidate = completedBySpan.get(span)
  const completed = completedCandidate !== undefined
    && completedCandidate.record.parent_span_id === parent
    && completedCandidate.record.event.action_id === started.record.event.action_id
    && completedCandidate.record.event.depth_index === started.record.event.depth_index
    && completedCandidate.sequence >= started.sequence ? completedCandidate : undefined
  claim(completed, claimed, raw)
  const event = completed?.record.event
  const status: DriftAttemptStatus = event === undefined
    ? "running"
    : !event.answer_present
      ? "incomplete"
      : event.answer_non_empty
        ? "completed"
        : "completed_empty"
  return {
    identity: `${started.record.event.depth_index}:${started.record.event.action_id}:${span}`,
    spanId: span,
    depthIndex: started.record.event.depth_index,
    actionId: started.record.event.action_id,
    query: started.record.event.query ?? null,
    context: context?.record.event.context ?? null,
    status,
    score: event?.score,
    answerPresent: event?.answer_present,
    answerNonEmpty: event?.answer_non_empty,
    followUpCount: event?.follow_up_count,
    targetActionIds: event?.target_action_ids ?? [],
    answer: event?.answer ?? null,
    followUpQueries: event?.follow_up_queries ?? null,
    rawEvents: [started, context, llm.started, llm.completed, completed].filter(defined).sort(bySequence),
    ...llm,
  }
}

function buildActionGraph(
  root: ExplainabilityEnvelope,
  primer: TypedEnvelope<DriftPrimerCompletedEvent> | undefined,
  attempts: readonly DriftActionAttemptView[],
): { nodes: DriftActionNodeView[]; edges: DriftEdgeView[] } {
  const nodeQueries = new Map<number, string | null>()
  const rootId = primer?.record.event.root_action_id
  if (rootId !== undefined) nodeQueries.set(rootId, root.record.event.query === undefined ? null : String(root.record.event.query))
  const edges: DriftEdgeView[] = []
  const appendEdges = (source: number, targets: readonly number[], queries: readonly string[] | null): void => {
    targets.forEach((target, index) => {
      edges.push({ sourceActionId: source, targetActionId: target, ordinal: edges.length })
      if (!nodeQueries.has(target)) nodeQueries.set(target, queries?.[index] ?? null)
    })
  }
  if (primer !== undefined) appendEdges(primer.record.event.root_action_id, primer.record.event.follow_up_action_ids, primer.record.event.follow_up_queries ?? null)
  for (const attempt of attempts) {
    if (!nodeQueries.has(attempt.actionId)) nodeQueries.set(attempt.actionId, attempt.query)
    appendEdges(attempt.actionId, attempt.targetActionIds, attempt.followUpQueries)
  }
  const attemptsByAction = new Map<number, DriftActionAttemptView[]>()
  for (const attempt of attempts) append(attemptsByAction, attempt.actionId, attempt)
  const ids = new Set<number>([...nodeQueries.keys(), ...attemptsByAction.keys()])
  return {
    edges,
    nodes: [...ids].sort((left, right) => left - right).map((actionId) => {
      const actionAttempts = attemptsByAction.get(actionId) ?? []
      const lastAttempt = actionAttempts.at(-1)
      return {
        actionId,
        query: nodeQueries.get(actionId) ?? lastAttempt?.query ?? null,
        status: actionId === rootId ? "root" : lastAttempt?.status ?? "not_explored",
        attempts: actionAttempts,
        outgoingEdges: edges.filter((edge) => edge.sourceActionId === actionId),
      }
    }),
  }
}

function stageLlm(
  anchor: ExplainabilityEnvelope | undefined,
  parent: string | undefined,
  startedBySpan: ReadonlyMap<string, TypedEnvelope<LlmRequestStartedEvent>>,
  completedBySpan: ReadonlyMap<string, TypedEnvelope<LlmRequestCompletedEvent>>,
  claimed: Set<number>,
  raw: ExplainabilityEnvelope[],
): DriftLlmView & { started?: TypedEnvelope<LlmRequestStartedEvent>; completed?: TypedEnvelope<LlmRequestCompletedEvent> } {
  const span = anchor?.record.span_id
  const startedCandidate = span === undefined ? undefined : startedBySpan.get(span)
  const started = startedCandidate !== undefined
    && startedCandidate.record.parent_span_id === parent
    && startedCandidate.sequence >= (anchor?.sequence ?? 0) ? startedCandidate : undefined
  const completedCandidate = span === undefined ? undefined : completedBySpan.get(span)
  const completed = completedCandidate !== undefined
    && completedCandidate.record.parent_span_id === parent
    && completedCandidate.sequence >= (started?.sequence ?? anchor?.sequence ?? 0) ? completedCandidate : undefined
  claim(started, claimed, raw)
  claim(completed, claimed, raw)
  return {
    started,
    completed,
    model: completed?.record.event.model_id ?? started?.record.event.model_id,
    inputTokens: completed?.record.event.input_tokens ?? started?.record.event.prompt_tokens,
    outputTokens: completed?.record.event.output_tokens,
    elapsedMs: completed?.record.event.elapsed_ms,
    exactPrompt: started?.record.event.prompt ?? null,
    rawResponse: completed?.record.event.response ?? null,
  }
}

function claim(envelope: ExplainabilityEnvelope | undefined, claimed: Set<number>, raw: ExplainabilityEnvelope[]): void {
  if (envelope === undefined || claimed.has(envelope.sequence)) return
  claimed.add(envelope.sequence)
  raw.push(envelope)
}

function append<K, V>(map: Map<K, V[]>, key: K, value: V): void {
  const values = map.get(key) ?? []
  values.push(value)
  map.set(key, values)
}

function typed<T extends ExplainabilityEventPayload>(envelope: ExplainabilityEnvelope, event: T): TypedEnvelope<T> {
  return { ...envelope, record: { ...envelope.record, event } }
}

function defined<T>(value: T | undefined): value is T { return value !== undefined }
function bySequence(left: ExplainabilityEnvelope, right: ExplainabilityEnvelope): number { return left.sequence - right.sequence }
function isObject(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null }
function isString(value: unknown): value is string { return typeof value === "string" }
function isOptionalString(value: unknown): value is string | undefined { return value === undefined || isString(value) }
function isBoolean(value: unknown): value is boolean { return typeof value === "boolean" }
function isFiniteNumber(value: unknown): value is number { return typeof value === "number" && Number.isFinite(value) }
function isNonNegativeInteger(value: unknown): value is number { return Number.isInteger(value) && Number(value) >= 0 }
function isStringArray(value: unknown): value is string[] { return Array.isArray(value) && value.every(isString) }
function isNumberArray(value: unknown): value is number[] { return Array.isArray(value) && value.every(isNonNegativeInteger) }
function isOptionalStringArray(value: unknown): value is string[] | undefined { return value === undefined || isStringArray(value) }
function hasUniqueNumbers(value: readonly number[]): boolean { return new Set(value).size === value.length }

function isDriftHydeStarted(event: ExplainabilityEventPayload): event is DriftHydeStartedEvent {
  return event.type === "drift_hyde_started" && isString(event.template_report_id) && isString(event.template_short_id)
    && isString(event.template_community_id) && isNonNegativeInteger(event.template_index) && isNonNegativeInteger(event.report_count)
    && event.report_count > 0 && event.template_index < event.report_count
}
function isDriftHydeCompleted(event: ExplainabilityEventPayload): event is DriftHydeCompletedEvent { return event.type === "drift_hyde_completed" && isBoolean(event.used_original_query) }
function isEmbeddingStarted(event: ExplainabilityEventPayload): event is EmbeddingStartedEvent { return event.type === "embedding_started" && isString(event.model_id) && isOptionalString(event.input) }
function isEmbeddingCompleted(event: ExplainabilityEventPayload): event is EmbeddingCompletedEvent { return event.type === "embedding_completed" && isString(event.model_id) && isNonNegativeInteger(event.prompt_tokens) && isNonNegativeInteger(event.dimensions) }
function isDriftReportsRanked(event: ExplainabilityEventPayload): event is DriftReportsRankedEvent {
  return event.type === "drift_reports_ranked" && Array.isArray(event.reports) && event.reports.every((report, index) => isObject(report)
    && isString(report.report_id) && isString(report.short_id) && isString(report.community_id)
    && isFiniteNumber(report.similarity) && report.rank === index + 1)
}
function isDriftPrimerStarted(event: ExplainabilityEventPayload): event is DriftPrimerStartedEvent { return event.type === "drift_primer_started" && isNonNegativeInteger(event.fold_count) && event.fold_count > 0 && isNonNegativeInteger(event.ranked_report_count) }
function isDriftPrimerFoldStarted(event: ExplainabilityEventPayload): event is DriftPrimerFoldStartedEvent { return event.type === "drift_primer_fold_started" && isNonNegativeInteger(event.fold_index) && isNonNegativeInteger(event.fold_count) && event.fold_count > event.fold_index && isStringArray(event.report_ids) }
function isDriftPrimerFoldCompleted(event: ExplainabilityEventPayload): event is DriftPrimerFoldCompletedEvent { return event.type === "drift_primer_fold_completed" && isNonNegativeInteger(event.fold_index) && isFiniteNumber(event.score) && isNonNegativeInteger(event.follow_up_count) && isOptionalString(event.intermediate_answer) && isOptionalStringArray(event.follow_up_queries) && (event.follow_up_queries === undefined || event.follow_up_queries.length === event.follow_up_count) }
function isDriftPrimerCompleted(event: ExplainabilityEventPayload): event is DriftPrimerCompletedEvent { return event.type === "drift_primer_completed" && isFiniteNumber(event.score) && isNonNegativeInteger(event.root_action_id) && isNonNegativeInteger(event.follow_up_count) && isNumberArray(event.follow_up_action_ids) && event.follow_up_action_ids.length === event.follow_up_count && isOptionalString(event.answer) && isOptionalStringArray(event.follow_up_queries) && (event.follow_up_queries === undefined || event.follow_up_queries.length === event.follow_up_count) }
function isDriftExplorationStarted(event: ExplainabilityEventPayload): event is DriftExplorationStartedEvent { return event.type === "drift_exploration_started" && isNonNegativeInteger(event.max_depth) && isNonNegativeInteger(event.selection_limit) && isNonNegativeInteger(event.root_action_id) }
function isDriftDepthActionsSelected(event: ExplainabilityEventPayload): event is DriftDepthActionsSelectedEvent {
  if (event.type !== "drift_depth_actions_selected"
    || !isNonNegativeInteger(event.depth_index)
    || !isNumberArray(event.candidate_action_ids)
    || !isNumberArray(event.selected_action_ids)
    || !isNonNegativeInteger(event.selection_limit)) return false
  const candidates = event.candidate_action_ids
  const selected = event.selected_action_ids
  return hasUniqueNumbers(candidates)
    && hasUniqueNumbers(selected)
    && selected.length <= event.selection_limit
    && selected.every((id) => candidates.includes(id))
}
function isDriftActionAttemptStarted(event: ExplainabilityEventPayload): event is DriftActionAttemptStartedEvent { return event.type === "drift_action_attempt_started" && isNonNegativeInteger(event.depth_index) && isNonNegativeInteger(event.action_id) && isOptionalString(event.query) }
function isDriftActionContextBuilt(event: ExplainabilityEventPayload): event is DriftActionContextBuiltEvent { return event.type === "drift_action_context_built" && isNonNegativeInteger(event.action_id) && isOptionalString(event.context) }
function isDriftActionAttemptCompleted(event: ExplainabilityEventPayload): event is DriftActionAttemptCompletedEvent { return event.type === "drift_action_attempt_completed" && isNonNegativeInteger(event.depth_index) && isNonNegativeInteger(event.action_id) && isBoolean(event.answer_present) && isBoolean(event.answer_non_empty) && (!event.answer_non_empty || event.answer_present) && (event.score === undefined || isFiniteNumber(event.score)) && isNonNegativeInteger(event.follow_up_count) && isNumberArray(event.target_action_ids) && event.target_action_ids.length === event.follow_up_count && isOptionalString(event.answer) && (event.answer === undefined || (event.answer_present && (event.answer.length > 0) === event.answer_non_empty)) && isOptionalStringArray(event.follow_up_queries) && (event.follow_up_queries === undefined || event.follow_up_queries.length === event.follow_up_count) }
function isDriftReduceContextBuilt(event: ExplainabilityEventPayload): event is DriftReduceContextBuiltEvent { return event.type === "drift_reduce_context_built" && isNonNegativeInteger(event.node_count) && isNonNegativeInteger(event.edge_count) && isNonNegativeInteger(event.included_answer_count) && isNumberArray(event.included_action_ids) && event.included_action_ids.length === event.included_answer_count && isOptionalString(event.state_context) && isOptionalString(event.reduce_context) }
function isLlmRequestStarted(event: ExplainabilityEventPayload): event is LlmRequestStartedEvent { return event.type === "llm_request_started" && isString(event.model_id) && isNonNegativeInteger(event.prompt_tokens) && isOptionalString(event.prompt) }
function isLlmRequestCompleted(event: ExplainabilityEventPayload): event is LlmRequestCompletedEvent { return event.type === "llm_request_completed" && isString(event.model_id) && isNonNegativeInteger(event.input_tokens) && isNonNegativeInteger(event.output_tokens) && isNonNegativeInteger(event.elapsed_ms) && isOptionalString(event.response) }
