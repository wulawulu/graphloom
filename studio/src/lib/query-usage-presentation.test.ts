import { describe, expect, it } from "vitest"

import type { StudioQueryUsage } from "@/api/types"
import { buildQueryUsagePresentation, humanizeCategory } from "@/lib/query-usage-presentation"

describe("Query usage presentation", () => {
  it("preserves the raw API usage contract while building presentation rows", () => {
    const usage: StudioQueryUsage = {
      llm_calls: 2,
      prompt_tokens: 11_183,
      output_tokens: 1_075,
      categories: {
        build_context: { llm_calls: 1, prompt_tokens: 17, output_tokens: 0 },
        response: { llm_calls: 1, prompt_tokens: 11_166, output_tokens: 1_075 },
      },
    }
    const before = structuredClone(usage)

    buildQueryUsagePresentation(usage, [])

    expect(usage).toEqual(before)
  })

  it("humanizes unknown categories without losing their words", () => {
    expect(humanizeCategory("future_map-response")).toBe("Future map response")
    expect(humanizeCategory("already readable")).toBe("Already readable")
  })
})
