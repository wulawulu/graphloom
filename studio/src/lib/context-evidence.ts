import type { ExplainabilityContextSection, ExplainabilityEnvelope } from "@/api/types"

export function latestContextSections(
  envelopes: readonly ExplainabilityEnvelope[],
): ExplainabilityContextSection[] {
  const sections = new Map<string, ExplainabilityContextSection>()
  const ordered = [...envelopes].sort((left, right) => left.sequence - right.sequence)

  for (const envelope of ordered) {
    const event = envelope.record.event
    if (event.type !== "context_section_built") continue
    const section = contextSection(event.section)
    if (section !== null) sections.set(`${section.section}:${section.name ?? ""}`, section)
  }

  return [...sections.values()]
}

function contextSection(value: unknown): ExplainabilityContextSection | null {
  if (typeof value !== "object" || value === null) return null
  const section = value as Partial<ExplainabilityContextSection>
  if (
    typeof section.section !== "string"
    || typeof section.token_budget !== "number"
    || typeof section.tokens_used !== "number"
    || typeof section.candidate_count !== "number"
    || typeof section.selected_count !== "number"
    || typeof section.truncated !== "boolean"
    || !Array.isArray(section.selected_record_ids)
    || !section.selected_record_ids.every((id) => typeof id === "string")
  ) return null
  return section as ExplainabilityContextSection
}
