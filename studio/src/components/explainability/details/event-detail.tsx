import type { TFunction } from "i18next"
import { useTranslation } from "react-i18next"

import type { ExplainabilityCandidate, ExplainabilityContextSection, ExplainabilityEventPayload } from "@/api/types"
import { CandidateTable } from "@/components/explainability/details/candidate-table"
import { DeveloperData } from "@/components/explainability/details/developer-data"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { describeEvent } from "@/lib/explainability"
import type { StudioTranslationKey } from "@/i18n/types"

interface EventDetailProps {
  event: ExplainabilityEventPayload
  onFocusGraph: (() => void) | null
}

const selectionFields: Record<string, string> = {
  entities_selected: "entities",
  relationships_selected: "relationships",
  community_reports_selected: "community_reports",
  covariates_selected: "covariates",
  text_units_selected: "text_units",
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

function asCandidates(value: unknown): ExplainabilityCandidate[] {
  if (!Array.isArray(value)) return []
  return value.filter((candidate): candidate is ExplainabilityCandidate => (
    isRecord(candidate)
    && typeof candidate.id === "string"
    && typeof candidate.record_type === "string"
    && typeof candidate.selected === "boolean"
  ))
}

function asStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : []
}

function numberValue(event: ExplainabilityEventPayload, key: string): number | null {
  return typeof event[key] === "number" ? event[key] : null
}

function stringValue(event: ExplainabilityEventPayload, key: string): string | null {
  return typeof event[key] === "string" ? event[key] : null
}

export function EventDetail({ event, onFocusGraph }: EventDetailProps): React.ReactElement {
  const { t } = useTranslation()
  let content: React.ReactNode
  if (event.type === "candidates_retrieved" || event.type === "candidates_filtered") {
    content = <CandidateTable candidates={asCandidates(event.candidates)} />
  } else if (selectionFields[event.type] !== undefined) {
    const field = selectionFields[event.type]
    const candidates = field === undefined ? [] : asCandidates(event[field])
    const descriptor = describeEvent(event)
    const label = descriptor.labelKey === null ? descriptor.rawLabel ?? event.type : t(descriptor.labelKey)
    content = <div className="space-y-3">{onFocusGraph !== null && (event.type === "entities_selected" || event.type === "relationships_selected") ? <Button size="sm" onClick={onFocusGraph}>{t("explainability.actions.focusAllInGraph")}</Button> : null}<CandidateTable candidates={candidates} label={label} /></div>
  } else {
    content = eventContent(t, event, onFocusGraph)
  }
  return <div className="space-y-3">{content}<DeveloperData event={event} /></div>
}

function eventContent(t: TFunction, event: ExplainabilityEventPayload, onFocusGraph: (() => void) | null): React.ReactNode {
  switch (event.type) {
    case "graph_expansion_started":
      return <GraphExpansionDetail ids={asStrings(event.seed_entity_ids)} onFocusGraph={onFocusGraph} />
    case "context_budget_allocated":
      return <ContextBudgetDetail event={event} />
    case "context_section_built":
      return <ContextSectionDetail value={event.section} />
    case "context_completed":
      return <ContextCompletedDetail event={event} />
    case "llm_request_started":
    case "llm_request_completed":
      return <LlmDetail event={event} />
    case "run_failed":
      return <KeyValues values={[["explainability.labels.status", t("explainability.messages.runFailed")], ["answer.labels.category", stringValue(event, "error_kind")], ["explainability.labels.diagnostic", stringValue(event, "message")]]} />
    case "run_started":
      return <KeyValues values={[["explainability.labels.kind", stringValue(event, "kind")], ["explainability.labels.contentMode", stringValue(event, "content_mode")]]} />
    case "run_completed":
      return <KeyValues values={[["explainability.labels.elapsed", withUnit(numberValue(event, "elapsed_ms"), "ms")]]} />
    case "query_started":
      return <ContentDetail label="navigation.query" metadata={[["explainability.labels.method", stringValue(event, "method")]]} content={stringValue(event, "query")} />
    case "mapping_query_built":
      return <ContentDetail label="explainability.labels.mappingQuery" metadata={[["explainability.labels.conversationTurns", numberValue(event, "conversation_turn_count")]]} content={stringValue(event, "mapping_query")} />
    case "embedding_started":
      return <ContentDetail label="explainability.labels.embeddingInput" metadata={modelIdentityMetadata(event)} content={stringValue(event, "input")} />
    case "embedding_completed":
      return <KeyValues values={[...modelIdentityMetadata(event), ["explainability.labels.inputTokens", numberValue(event, "prompt_tokens")], ["explainability.labels.dimensions", numberValue(event, "dimensions")]]} />
    case "warning":
      return <KeyValues values={[["explainability.labels.code", stringValue(event, "code")], ["explainability.labels.message", stringValue(event, "message")], ["explainability.labels.record", stringValue(event, "record_id")]]} />
    default:
      return <GenericEventDetail event={event} />
  }
}

function GraphExpansionDetail({ ids, onFocusGraph }: { ids: string[]; onFocusGraph: (() => void) | null }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="space-y-3"><KeyValues values={[["explainability.labels.seedEntities", ids.length]]} />{ids.length > 0 ? <div className="flex flex-wrap gap-1">{ids.map((id) => <Badge key={id} variant="outline">{id}</Badge>)}</div> : null}{onFocusGraph !== null ? <Button size="sm" onClick={onFocusGraph}>{t("explainability.actions.focusExpansionInGraph")}</Button> : null}</div>
}

function ContextBudgetDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const total = numberValue(event, "total_token_budget") ?? 0
  const sections = Array.isArray(event.sections) ? event.sections.filter(isRecord) : []
  return <div className="space-y-3"><KeyValues values={[["explainability.labels.totalTokenBudget", total]]} /><div className="space-y-2">{sections.map((section, index) => { const budget = typeof section.token_budget === "number" ? section.token_budget : 0; const percent = total > 0 ? Math.min((budget / total) * 100, 100) : 0; return <div key={`${String(section.section)}:${index}`}><div className="mb-1 flex justify-between text-xs"><span>{String(section.section).replaceAll("_", " ")}</span><span>{budget}</span></div><div className="h-2 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} /></div></div> })}</div></div>
}

function ContextSectionDetail({ value }: { value: unknown }): React.ReactElement {
  const { t } = useTranslation()
  if (!isRecord(value)) return <p className="text-xs text-muted-foreground">{t("explainability.messages.noSectionMetadata")}</p>
  const section = value as unknown as ExplainabilityContextSection
  const ids = asStrings(section.selected_record_ids)
  return <div className="space-y-3"><KeyValues values={[["explainability.labels.section", typeof section.section === "string" ? section.section : null], ["explainability.labels.name", typeof section.name === "string" ? section.name : null], ["answer.labels.tokens", typeof section.tokens_used === "number" && typeof section.token_budget === "number" ? `${section.tokens_used} / ${section.token_budget}` : null], ["explainability.labels.records", typeof section.selected_count === "number" && typeof section.candidate_count === "number" ? `${section.selected_count} / ${section.candidate_count}` : null], ["explainability.labels.truncated", typeof section.truncated === "boolean" ? t(section.truncated ? "explainability.labels.yes" : "explainability.labels.no") : null]]} />{ids.length > 0 ? <div className="flex flex-wrap gap-1">{ids.map((id) => <Badge key={id} variant="outline">{id}</Badge>)}</div> : null}</div>
}

function ContextCompletedDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const { t } = useTranslation()
  const context = stringValue(event, "context")
  return <div className="space-y-3"><KeyValues values={[["explainability.labels.tokensUsed", numberValue(event, "tokens_used")]]} />{context === null ? <p className="text-xs text-muted-foreground">{t("explainability.messages.contentHiddenByExplainabilityMode")}</p> : <ContentBlock label="explainability.labels.contextPreview" value={context} />}</div>
}

function LlmDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const { t } = useTranslation()
  const prompt = stringValue(event, "prompt")
  const response = stringValue(event, "response")
  return <div className="space-y-3"><KeyValues values={[...modelIdentityMetadata(event), ["explainability.labels.promptTokens", numberValue(event, "prompt_tokens")], ["explainability.labels.inputTokens", numberValue(event, "input_tokens")], ["explainability.labels.outputTokens", numberValue(event, "output_tokens")], ["explainability.labels.latency", withUnit(numberValue(event, "elapsed_ms"), "ms")]]} />{prompt !== null ? <ContentBlock label="explainability.labels.prompt" value={prompt} /> : null}{response !== null ? <ContentBlock label="explainability.labels.response" value={response} /> : null}{prompt === null && response === null ? <p className="text-xs text-muted-foreground">{t("explainability.messages.contentHiddenByExplainabilityMode")}</p> : null}</div>
}

function modelIdentityMetadata(event: ExplainabilityEventPayload): Array<[StudioTranslationKey, unknown]> {
  return [
    ["explainability.labels.modelConfigId", stringValue(event, "model_id")],
    ["explainability.labels.modelName", stringValue(event, "model_name") ?? stringValue(event, "model_id")],
    ["explainability.labels.provider", stringValue(event, "provider")],
  ]
}

function ContentDetail({ label, metadata, content }: { label: StudioTranslationKey; metadata: Array<[StudioTranslationKey, unknown]>; content: string | null }): React.ReactElement {
  const { t } = useTranslation()
  return <div className="space-y-3"><KeyValues values={metadata} />{content === null ? <p className="text-xs text-muted-foreground">{t("explainability.messages.contentHiddenByExplainabilityMode")}</p> : <ContentBlock label={label} value={content} />}</div>
}

function ContentBlock({ label, value }: { label: StudioTranslationKey; value: string }): React.ReactElement {
  const { t } = useTranslation()
  return <details className="rounded-md border p-3"><summary className="cursor-pointer text-xs font-medium">{t(label)}</summary><pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-5">{value}</pre></details>
}

function GenericEventDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const { t } = useTranslation()
  const values = Object.entries(event).filter(([key, value]) => key !== "type" && ["string", "number", "boolean"].includes(typeof value))
  return <div className="space-y-2"><p className="text-xs font-medium">{t("explainability.labels.eventDetails")}</p><KeyValues values={values.map(([key, value]) => [key, value])} translateLabels={false} /></div>
}

type KeyValuesProps =
  | { values: Array<[StudioTranslationKey, unknown]>; translateLabels?: true }
  | { values: Array<[string, unknown]>; translateLabels: false }

function KeyValues(props: KeyValuesProps): React.ReactElement {
  const { t } = useTranslation()
  if (props.translateLabels === false) return <KeyValueList values={props.values} />
  return <KeyValueList values={props.values.map(([label, value]) => [t(label), value])} />
}

function KeyValueList({ values }: { values: Array<[string, unknown]> }): React.ReactElement {
  const present = values.filter(([, value]) => value !== null && value !== undefined)
  return <dl className="grid grid-cols-[minmax(7rem,auto)_1fr] gap-x-3 gap-y-1 text-xs">{present.map(([label, value]) => <div key={label} className="contents"><dt className="text-muted-foreground capitalize">{label}</dt><dd className="min-w-0 whitespace-pre-wrap">{String(value)}</dd></div>)}</dl>
}

function withUnit(value: number | null, unit: string): string | null {
  return value === null ? null : `${value} ${unit}`
}
