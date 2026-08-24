import { createContext, useContext } from "react"

import type { GraphTextUnitRef } from "@/api/types"

export type TextUnitEvidenceStatus = "idle" | "loading" | "resolved" | "unavailable"

export interface TextUnitEvidenceContextValue {
  refs: ReadonlyMap<string, GraphTextUnitRef>
  resolve: (ids: readonly string[]) => void
  status: (id: string) => TextUnitEvidenceStatus
}

const unavailableEvidence: TextUnitEvidenceContextValue = {
  refs: new Map(),
  resolve: () => undefined,
  status: () => "unavailable",
}

export const TextUnitEvidenceContext = createContext<TextUnitEvidenceContextValue | null>(null)

export function useTextUnitEvidence(): TextUnitEvidenceContextValue {
  return useContext(TextUnitEvidenceContext) ?? unavailableEvidence
}
