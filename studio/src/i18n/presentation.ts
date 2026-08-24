import type { TFunction } from "i18next"

import type { ExplainabilityEventPayload } from "@/api/types"

export function semanticStepTitle(t: TFunction, kind: string): string {
  const titles: Record<string, string> = {
    "entity-mapping": "Entity Mapping",
    "graph-expansion": "Graph Expansion",
    "context-assembly": "Context Assembly",
    "answer-generation": "Answer Generation",
    "text-retrieval": "Text Retrieval",
    "basic-context-assembly": "Context Assembly",
    "basic-answer-generation": "Answer Generation",
    "community-selection": "Community Selection",
    "community-context": "Community Context",
    "map-analysis": "Map Analysis",
    "evidence-reduction": "Evidence Reduction",
    "global-answer-generation": "Answer Generation",
    "drift-primer-ranking": "Primer & Ranking",
    "drift-exploration": "Exploration",
    "drift-final-synthesis": "Final Synthesis",
  }
  return t(titles[kind] ?? kind)
}

export function localizedEventSummary(t: TFunction, event: ExplainabilityEventPayload): string {
  const numberValue = (field: string, label: string): string | null =>
    typeof event[field] === "number" ? `${t(label)}: ${String(event[field])}` : null
  const model = typeof event.model_id === "string" ? `${t("Model")}: ${event.model_id}` : null
  const section = typeof event.section === "string"
    ? `${t("Section")}: ${event.section}`
    : typeof event.section === "object" && event.section !== null && "section" in event.section
      ? `${t("Section")}: ${String(event.section.section)}`
      : null
  const collectionLabels: Record<string, string> = {
    entities: "Entities",
    relationships: "Relationships",
    community_reports: "Community reports",
    covariates: "Covariates",
    text_units: "Text units",
    candidates: "Candidates",
  }
  const candidates = Object.entries(collectionLabels)
    .map(([field, label]) => Array.isArray(event[field]) ? `${t(label)}: ${String(event[field].length)}` : null)
    .find((value) => value !== null)

  return [
    model,
    section,
    candidates,
    numberValue("elapsed_ms", "Elapsed (ms)"),
    numberValue("tokens_used", "Tokens used"),
    numberValue("input_tokens", "Input tokens"),
    numberValue("output_tokens", "Output tokens"),
  ].filter((value): value is string => value !== null).join(" · ")
}
