import type { ExplainabilityEnvelope, StudioQueryUsage } from "@/api/types"
import type { StudioTranslationKey } from "@/i18n/types"
import { buildSemanticTimeline } from "@/lib/semantic-timeline"

export interface QueryUsageRowPresentation {
  rawCategory: string
  stageKey?: StudioTranslationKey
  stageFallback: string
  operationKey: StudioTranslationKey
  calls: number
  inputTokens: number
  outputTokens: number
  outputApplicable: boolean
  model?: string
}

export interface QueryUsagePresentation {
  rows: QueryUsageRowPresentation[]
  totalOperations: number
  basicOperations?: {
    embedding: number
    generation: number
  }
}

export function buildQueryUsagePresentation(
  usage: StudioQueryUsage,
  envelopes: readonly ExplainabilityEnvelope[],
): QueryUsagePresentation {
  const timeline = buildSemanticTimeline(envelopes)
  const basicModels = timeline.method === "basic" ? basicOperationModels(timeline.steps) : undefined
  const categoryRows = Object.entries(usage.categories).map(([rawCategory, category]) => {
    if (timeline.method === "basic" && rawCategory === "build_context") {
      return {
        rawCategory,
        stageKey: "answer.usage.textRetrieval" as const,
        stageFallback: humanizeCategory(rawCategory),
        operationKey: "answer.usage.embedding" as const,
        calls: category.llm_calls,
        inputTokens: category.prompt_tokens,
        outputTokens: category.output_tokens,
        outputApplicable: false,
        model: basicModels?.embedding,
      }
    }
    if (timeline.method === "basic" && rawCategory === "response") {
      return {
        rawCategory,
        stageKey: "answer.usage.answerGeneration" as const,
        stageFallback: humanizeCategory(rawCategory),
        operationKey: "answer.usage.textGeneration" as const,
        calls: category.llm_calls,
        inputTokens: category.prompt_tokens,
        outputTokens: category.output_tokens,
        outputApplicable: true,
        model: basicModels?.generation,
      }
    }
    return {
      rawCategory,
      stageFallback: humanizeCategory(rawCategory),
      operationKey: "answer.usage.modelOperation" as const,
      calls: category.llm_calls,
      inputTokens: category.prompt_tokens,
      outputTokens: category.output_tokens,
      outputApplicable: true,
    }
  })
  const buildContext = timeline.method === "basic" ? usage.categories.build_context : undefined
  const response = timeline.method === "basic" ? usage.categories.response : undefined
  const rows = categoryRows.length > 0 ? categoryRows : [{
    rawCategory: "",
    stageKey: "answer.usage.total" as const,
    stageFallback: "Total",
    operationKey: "answer.usage.modelOperation" as const,
    calls: usage.llm_calls,
    inputTokens: usage.prompt_tokens,
    outputTokens: usage.output_tokens,
    outputApplicable: true,
  }]
  return {
    rows,
    totalOperations: usage.llm_calls,
    ...(buildContext === undefined || response === undefined ? {} : {
      basicOperations: {
        embedding: buildContext.llm_calls,
        generation: response.llm_calls,
      },
    }),
  }
}

export function humanizeCategory(category: string): string {
  const words = category.trim().replaceAll(/[_-]+/g, " ").replaceAll(/\s+/g, " ")
  if (words.length === 0) return category
  return `${words[0]?.toUpperCase() ?? ""}${words.slice(1)}`
}

function basicOperationModels(steps: ReturnType<typeof buildSemanticTimeline>["steps"]): { embedding?: string; generation?: string } {
  const retrieval = steps.find((step) => step.kind === "text-retrieval")
  const answer = steps.find((step) => step.kind === "basic-answer-generation")
  return {
    ...(retrieval?.kind === "text-retrieval" && retrieval.summary.model !== undefined ? { embedding: retrieval.summary.model } : {}),
    ...(answer?.kind === "basic-answer-generation" && answer.summary.model !== undefined ? { generation: answer.summary.model } : {}),
  }
}
