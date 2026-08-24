import type { ContentMode } from "@/api/types"

export const explainabilityDetailOptions: ContentMode[] = ["metadata", "content", "debug"]

export type ResponseStyle = "standard" | "concise" | "detailed" | "custom"

export interface ResponseStyleMetadata {
  responseType: string | null
}

export const responseStyles: Record<ResponseStyle, ResponseStyleMetadata> = {
  standard: {
    responseType: "Multiple Paragraphs",
  },
  concise: {
    responseType: "Single Paragraph",
  },
  detailed: {
    responseType: "A detailed answer with sections and multiple paragraphs",
  },
  custom: {
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
