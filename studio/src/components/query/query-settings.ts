import type { ContentMode } from "@/api/types"

export interface ExplainabilityDetailMetadata {
  label: string
  description: string
}

export const explainabilityDetails: Record<ContentMode, ExplainabilityDetailMetadata> = {
  metadata: {
    label: "Standard",
    description: "Records identifiers, scores, token usage and timing without storing full query, Context, Prompt or model content.",
  },
  content: {
    label: "Detailed",
    description: "Also records permitted full query, Context, Prompt and model response content for inspecting how the answer was produced.",
  },
  debug: {
    label: "Debug",
    description: "Records the most verbose supported diagnostic information for development and troubleshooting.",
  },
}

export type ResponseStyle = "standard" | "concise" | "detailed" | "custom"

export interface ResponseStyleMetadata {
  label: string
  description: string
  responseType: string | null
}

export const responseStyles: Record<ResponseStyle, ResponseStyleMetadata> = {
  standard: {
    label: "Standard",
    description: "A balanced answer using multiple paragraphs.",
    responseType: "Multiple Paragraphs",
  },
  concise: {
    label: "Concise",
    description: "A shorter answer in a single paragraph.",
    responseType: "Single Paragraph",
  },
  detailed: {
    label: "Detailed",
    description: "A more detailed answer organized into sections and paragraphs.",
    responseType: "A detailed answer with sections and multiple paragraphs",
  },
  custom: {
    label: "Custom",
    description: "Use your own response-length and formatting instruction.",
    responseType: null,
  },
}

export const responseStyleOptions = Object.entries(responseStyles) as Array<[ResponseStyle, ResponseStyleMetadata]>

export const MAX_RESPONSE_TYPE_BYTES = 256

export function responseTypeForStyle(style: ResponseStyle, customResponse: string): string {
  return responseStyles[style].responseType ?? customResponse
}

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength
}
