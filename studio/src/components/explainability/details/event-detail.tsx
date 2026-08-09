import type { ExplainabilityCandidate, ExplainabilityContextSection, ExplainabilityEventPayload } from "@/api/types"
import { CandidateTable } from "@/components/explainability/details/candidate-table"
import { DeveloperData } from "@/components/explainability/details/developer-data"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

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
  let content: React.ReactNode
  if (event.type === "candidates_retrieved" || event.type === "candidates_filtered") {
    content = <CandidateTable candidates={asCandidates(event.candidates)} />
  } else if (selectionFields[event.type] !== undefined) {
    const field = selectionFields[event.type]
    const candidates = field === undefined ? [] : asCandidates(event[field])
    content = <div className="space-y-3">{onFocusGraph !== null && (event.type === "entities_selected" || event.type === "relationships_selected") ? <Button size="sm" onClick={onFocusGraph}>Focus all in graph</Button> : null}<CandidateTable candidates={candidates} label={event.type.replaceAll("_", " ")} /></div>
  } else {
    content = eventContent(event, onFocusGraph)
  }
  return <div className="space-y-3">{content}<DeveloperData event={event} /></div>
}

function eventContent(event: ExplainabilityEventPayload, onFocusGraph: (() => void) | null): React.ReactNode {
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
      return <KeyValues values={[["Status", "Run failed."], ["Category", stringValue(event, "error_kind")], ["Diagnostic", stringValue(event, "message")]]} />
    case "run_started":
      return <KeyValues values={[["Kind", stringValue(event, "kind")], ["Content mode", stringValue(event, "content_mode")]]} />
    case "run_completed":
      return <KeyValues values={[["Elapsed", withUnit(numberValue(event, "elapsed_ms"), "ms")]]} />
    case "query_started":
      return <ContentDetail label="Query" metadata={[["Method", stringValue(event, "method")]]} content={stringValue(event, "query")} />
    case "mapping_query_built":
      return <ContentDetail label="Mapping query" metadata={[["Conversation turns", numberValue(event, "conversation_turn_count")]]} content={stringValue(event, "mapping_query")} />
    case "embedding_started":
      return <ContentDetail label="Embedding input" metadata={[["Model", stringValue(event, "model_id")]]} content={stringValue(event, "input")} />
    case "embedding_completed":
      return <KeyValues values={[["Model", stringValue(event, "model_id")], ["Input tokens", numberValue(event, "prompt_tokens")], ["Dimensions", numberValue(event, "dimensions")]]} />
    case "warning":
      return <KeyValues values={[["Code", stringValue(event, "code")], ["Message", stringValue(event, "message")], ["Record", stringValue(event, "record_id")]]} />
    default:
      return <GenericEventDetail event={event} />
  }
}

function GraphExpansionDetail({ ids, onFocusGraph }: { ids: string[]; onFocusGraph: (() => void) | null }): React.ReactElement {
  return <div className="space-y-3"><KeyValues values={[["Seed entities", ids.length]]} />{ids.length > 0 ? <div className="flex flex-wrap gap-1">{ids.map((id) => <Badge key={id} variant="outline">{id}</Badge>)}</div> : null}{onFocusGraph !== null ? <Button size="sm" onClick={onFocusGraph}>Focus expansion in graph</Button> : null}</div>
}

function ContextBudgetDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const total = numberValue(event, "total_token_budget") ?? 0
  const sections = Array.isArray(event.sections) ? event.sections.filter(isRecord) : []
  return <div className="space-y-3"><KeyValues values={[["Total token budget", total]]} /><div className="space-y-2">{sections.map((section, index) => { const budget = typeof section.token_budget === "number" ? section.token_budget : 0; const percent = total > 0 ? Math.min((budget / total) * 100, 100) : 0; return <div key={`${String(section.section)}:${index}`}><div className="mb-1 flex justify-between text-xs"><span>{String(section.section).replaceAll("_", " ")}</span><span>{budget}</span></div><div className="h-2 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} /></div></div> })}</div></div>
}

function ContextSectionDetail({ value }: { value: unknown }): React.ReactElement {
  if (!isRecord(value)) return <p className="text-xs text-muted-foreground">No section metadata.</p>
  const section = value as unknown as ExplainabilityContextSection
  const ids = asStrings(section.selected_record_ids)
  return <div className="space-y-3"><KeyValues values={[["Section", typeof section.section === "string" ? section.section : null], ["Name", typeof section.name === "string" ? section.name : null], ["Tokens", typeof section.tokens_used === "number" && typeof section.token_budget === "number" ? `${section.tokens_used} / ${section.token_budget}` : null], ["Records", typeof section.selected_count === "number" && typeof section.candidate_count === "number" ? `${section.selected_count} / ${section.candidate_count}` : null], ["Truncated", typeof section.truncated === "boolean" ? (section.truncated ? "Yes" : "No") : null]]} />{ids.length > 0 ? <div className="flex flex-wrap gap-1">{ids.map((id) => <Badge key={id} variant="outline">{id}</Badge>)}</div> : null}</div>
}

function ContextCompletedDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const context = stringValue(event, "context")
  return <div className="space-y-3"><KeyValues values={[["Tokens used", numberValue(event, "tokens_used")]]} />{context === null ? <p className="text-xs text-muted-foreground">Content hidden by explainability mode.</p> : <ContentBlock label="Context preview" value={context} />}</div>
}

function LlmDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const prompt = stringValue(event, "prompt")
  const response = stringValue(event, "response")
  return <div className="space-y-3"><KeyValues values={[["Model", stringValue(event, "model_id")], ["Prompt tokens", numberValue(event, "prompt_tokens")], ["Input tokens", numberValue(event, "input_tokens")], ["Output tokens", numberValue(event, "output_tokens")], ["Latency", withUnit(numberValue(event, "elapsed_ms"), "ms")]]} />{prompt !== null ? <ContentBlock label="Prompt" value={prompt} /> : null}{response !== null ? <ContentBlock label="Response" value={response} /> : null}{prompt === null && response === null ? <p className="text-xs text-muted-foreground">Content hidden by explainability mode.</p> : null}</div>
}

function ContentDetail({ label, metadata, content }: { label: string; metadata: Array<[string, unknown]>; content: string | null }): React.ReactElement {
  return <div className="space-y-3"><KeyValues values={metadata} />{content === null ? <p className="text-xs text-muted-foreground">Content hidden by explainability mode.</p> : <ContentBlock label={label} value={content} />}</div>
}

function ContentBlock({ label, value }: { label: string; value: string }): React.ReactElement {
  return <details className="rounded-md border p-3"><summary className="cursor-pointer text-xs font-medium">{label}</summary><pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-5">{value}</pre></details>
}

function GenericEventDetail({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  const values = Object.entries(event).filter(([key, value]) => key !== "type" && ["string", "number", "boolean"].includes(typeof value))
  return <div className="space-y-2"><p className="text-xs font-medium">Event details</p><KeyValues values={values.map(([key, value]) => [key.replaceAll("_", " "), value])} /></div>
}

function KeyValues({ values }: { values: Array<[string, unknown]> }): React.ReactElement {
  const present = values.filter(([, value]) => value !== null && value !== undefined)
  return <dl className="grid grid-cols-[minmax(7rem,auto)_1fr] gap-x-3 gap-y-1 text-xs">{present.map(([label, value]) => <div key={label} className="contents"><dt className="text-muted-foreground capitalize">{label}</dt><dd className="min-w-0 whitespace-pre-wrap">{String(value)}</dd></div>)}</dl>
}

function withUnit(value: number | null, unit: string): string | null {
  return value === null ? null : `${value} ${unit}`
}
