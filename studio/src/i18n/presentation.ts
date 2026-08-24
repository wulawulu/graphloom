import type { TFunction } from "i18next"

import type { ExplainabilityEventPayload } from "@/api/types"
import type { StudioTranslationKey } from "@/i18n/types"

const SEMANTIC_STEP_TITLE_KEYS: Readonly<Record<string, StudioTranslationKey>> = {
  "entity-mapping": "explainability.steps.entityMapping",
  "graph-expansion": "explainability.steps.graphExpansion",
  "context-assembly": "explainability.steps.contextAssembly",
  "answer-generation": "explainability.steps.answerGeneration",
  "text-retrieval": "explainability.steps.textRetrieval",
  "basic-context-assembly": "explainability.steps.contextAssembly",
  "basic-answer-generation": "explainability.steps.answerGeneration",
  "community-selection": "explainability.steps.communitySelection",
  "community-context": "explainability.steps.communityContext",
  "map-analysis": "explainability.steps.mapAnalysis",
  "evidence-reduction": "explainability.steps.evidenceReduction",
  "global-answer-generation": "explainability.steps.answerGeneration",
  "drift-primer-ranking": "explainability.steps.primerRanking",
  "drift-exploration": "explainability.steps.exploration",
  "drift-final-synthesis": "explainability.steps.finalSynthesis",
}

const CONTEXT_SECTION_TITLE_KEYS: Readonly<Record<string, StudioTranslationKey>> = {
  entities: "explainability.labels.entities",
  relationships: "explainability.labels.relationships",
  community_reports: "explainability.labels.communityReports",
  covariates: "explainability.labels.covariates",
  text_units: "explainability.labels.textUnits",
}

export function semanticStepTitle(t: TFunction, kind: string): string {
  const key = SEMANTIC_STEP_TITLE_KEYS[kind]
  return key === undefined ? humanizeWireValue(kind) : t(key)
}

export function contextSectionTitle(t: TFunction, section: string, name?: string): string {
  if (name !== undefined) return name
  const key = CONTEXT_SECTION_TITLE_KEYS[section]
  return key === undefined ? humanizeWireValue(section) : t(key)
}

export function localizedEventSummary(t: TFunction, event: ExplainabilityEventPayload): string {
  const numberValue = (field: string, labelKey: StudioTranslationKey): string | null =>
    typeof event[field] === "number" ? `${t(labelKey)}: ${String(event[field])}` : null
  const model = typeof event.model_id === "string" ? `${t("explainability.labels.model")}: ${event.model_id}` : null
  const section = typeof event.section === "string"
    ? `${t("explainability.labels.section")}: ${event.section}`
    : typeof event.section === "object" && event.section !== null && "section" in event.section
      ? `${t("explainability.labels.section")}: ${String(event.section.section)}`
      : null
  const collectionLabelKeys: Readonly<Record<string, StudioTranslationKey>> = {
    entities: "explainability.labels.entities",
    relationships: "explainability.labels.relationships",
    community_reports: "explainability.labels.communityReports",
    covariates: "explainability.labels.covariates",
    text_units: "explainability.labels.textUnits",
    candidates: "explainability.labels.candidates",
  }
  const candidates = Object.entries(collectionLabelKeys)
    .map(([field, labelKey]) => Array.isArray(event[field]) ? `${t(labelKey)}: ${String(event[field].length)}` : null)
    .find((value) => value !== null)

  return [
    model,
    section,
    candidates,
    numberValue("elapsed_ms", "explainability.labels.elapsedMs"),
    numberValue("tokens_used", "explainability.labels.tokensUsed"),
    numberValue("input_tokens", "explainability.labels.inputTokens"),
    numberValue("output_tokens", "explainability.labels.outputTokens"),
  ].filter((value): value is string => value !== null).join(" · ")
}

function humanizeWireValue(value: string): string {
  return value.replaceAll("_", " ").replaceAll("-", " ")
}
