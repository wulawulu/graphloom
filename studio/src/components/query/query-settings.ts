export const DEVELOPER_MODE_STORAGE_KEY = "graphloom.studio.developerMode"

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

export function readDeveloperMode(): boolean {
  if (typeof window === "undefined") return false
  try {
    return window.localStorage.getItem(DEVELOPER_MODE_STORAGE_KEY) === "true"
  } catch {
    return false
  }
}

export function writeDeveloperMode(enabled: boolean): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(DEVELOPER_MODE_STORAGE_KEY, String(enabled))
  } catch {
    // Storage can be unavailable in privacy-restricted browser contexts.
  }
}
