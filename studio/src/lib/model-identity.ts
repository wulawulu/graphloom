import type { GenericExplainabilityEventPayload } from "@/api/types"

/** Resolve the effective provider model while preserving historical event compatibility. */
export function effectiveModelName(event: GenericExplainabilityEventPayload | undefined): string | undefined {
  if (event === undefined) return undefined
  return stringField(event.model_name) ?? stringField(event.model_id)
}

function stringField(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined
}
