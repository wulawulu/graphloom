import { useEffect, useState, type SyntheticEvent } from "react"
import { LoaderCircle, Play, SlidersHorizontal } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { ApiError, startQuery } from "@/api/client"
import type { ContentMode, StartQueryMethod, StartQueryResponse } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import {
  explainabilityDetailOptions,
  MAX_RESPONSE_TYPE_BYTES,
  responseStyleOptions,
  responseTypeForStyle,
  type ResponseStyle,
  utf8ByteLength,
} from "@/components/query/query-settings"

interface QueryComposerProps {
  onAccepted: (response: StartQueryResponse, submittedQuery: string) => void
  resetRevision: number
}

type StudioQueryMode = "basic" | "local" | "global" | "dynamic-global" | "drift"

interface QueryModeMetadata {
  method: StartQueryMethod
  dynamicCommunitySelection: boolean
}

const queryModes: Record<StudioQueryMode, QueryModeMetadata> = {
  basic: { method: "basic", dynamicCommunitySelection: false },
  local: { method: "local", dynamicCommunitySelection: false },
  global: { method: "global", dynamicCommunitySelection: false },
  "dynamic-global": { method: "global", dynamicCommunitySelection: true },
  drift: { method: "drift", dynamicCommunitySelection: false },
}

const queryModeOptions = Object.entries(queryModes) as Array<[StudioQueryMode, QueryModeMetadata]>

export function QueryComposer({ onAccepted, resetRevision }: QueryComposerProps): React.ReactElement {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const [queryMode, setQueryMode] = useState<StudioQueryMode>("local")
  const [contentMode, setContentMode] = useState<ContentMode>("metadata")
  const [responseStyle, setResponseStyle] = useState<ResponseStyle>("standard")
  const [customResponse, setCustomResponse] = useState("")
  const [submitting, setSubmitting] = useState(false)

  useEffect(() => setQuery(""), [resetRevision])

  const submit = async (event: SyntheticEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    const responseType = responseTypeForStyle(responseStyle, customResponse)
    if (query.length === 0 || submitting || responseType.length === 0 || utf8ByteLength(responseType) > MAX_RESPONSE_TYPE_BYTES) return
    setSubmitting(true)
    try {
      const mode = queryModes[queryMode]
      const response = await startQuery({
        query,
        method: mode.method,
        dynamic_community_selection: mode.dynamicCommunitySelection,
        content_mode: contentMode,
        response_type: responseType,
      })
      onAccepted(response, query)
      toast.success(t("{{method}} Query accepted", { method: t(queryModeLabelKey(queryMode)) }))
    } catch (error) {
      const message = error instanceof ApiError && error.status === 429
        ? t("Studio is at its active Query limit. Try again after a Run finishes.")
        : t("The Query could not be accepted.")
      toast.error(message)
    } finally {
      setSubmitting(false)
    }
  }

  const modeLabel = t(queryModeLabelKey(queryMode))
  const responseType = responseTypeForStyle(responseStyle, customResponse)
  const customResponseError = responseStyle === "custom" && responseType.length === 0
    ? t("Custom response instructions are required.")
    : responseStyle === "custom" && utf8ByteLength(responseType) > MAX_RESPONSE_TYPE_BYTES
      ? t("Custom response instructions must be {{count}} bytes or fewer.", { count: MAX_RESPONSE_TYPE_BYTES })
      : null

  return (
    <form className="rounded-lg border bg-card shadow-sm" onSubmit={(event) => void submit(event)}>
      <div className="p-2 pb-0">
        <label htmlFor="studio-query" className="sr-only">{t("Ask about the graph")}</label>
        <Textarea
          id="studio-query"
          className="min-h-20 resize-none border-0 bg-transparent px-1 shadow-none focus-visible:ring-0"
          value={query}
          maxLength={1024 * 1024}
          placeholder={t("Ask a question about the indexed graph…")}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      <Collapsible>
        <CollapsibleContent className="space-y-3 border-t px-3 py-3">
          <div className="space-y-1.5">
            <label htmlFor="content-mode" className="text-xs font-medium text-muted-foreground">{t("Explainability detail")}</label>
            <Select value={contentMode} onValueChange={(value) => setContentMode(value as ContentMode)}>
              <SelectTrigger id="content-mode"><SelectValue /></SelectTrigger>
              <SelectContent>{explainabilityDetailOptions.map((value) => <SelectItem key={value} value={value}>{t(explainabilityLabelKey(value))}</SelectItem>)}</SelectContent>
            </Select>
            <p className="text-[11px] leading-4 text-muted-foreground">{t(explainabilityDescriptionKey(contentMode))}</p>
          </div>
          <div className="space-y-1.5">
            <label htmlFor="response-style" className="text-xs font-medium text-muted-foreground">{t("Response style")}</label>
            <Select value={responseStyle} onValueChange={(value) => setResponseStyle(value as ResponseStyle)}>
              <SelectTrigger id="response-style"><SelectValue /></SelectTrigger>
              <SelectContent>{responseStyleOptions.map(([value]) => <SelectItem key={value} value={value}>{t(responseStyleLabelKey(value))}</SelectItem>)}</SelectContent>
            </Select>
            <p className="text-[11px] leading-4 text-muted-foreground">{t(responseStyleDescriptionKey(responseStyle))}</p>
            {responseStyle === "custom" ? (
              <div className="space-y-1.5 pt-1">
                <label htmlFor="custom-response" className="text-xs font-medium text-muted-foreground">{t("Custom response instructions")}</label>
                <Input
                  id="custom-response"
                  value={customResponse}
                  aria-invalid={customResponseError !== null}
                  aria-describedby={customResponseError === null ? undefined : "custom-response-error"}
                  placeholder={t("For example: Answer in five bullet points.")}
                  onChange={(event) => setCustomResponse(event.target.value)}
                />
                {customResponseError === null ? null : <p id="custom-response-error" className="text-[11px] leading-4 text-destructive">{customResponseError}</p>}
              </div>
            ) : null}
          </div>
        </CollapsibleContent>
        <div className="flex items-center justify-between border-t px-2 py-1.5">
          <div className="flex items-center gap-1">
            <Select disabled={submitting} value={queryMode} onValueChange={(value) => setQueryMode(value as StudioQueryMode)}>
              <SelectTrigger aria-label={t("Query method")} className="h-6 w-auto gap-1 px-2 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {queryModeOptions.map(([value]) => (
                  <SelectItem key={value} value={value}>{t(queryModeLabelKey(value))}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Badge variant="outline">{t(explainabilityLabelKey(contentMode))}</Badge>
          </div>
          <div className="flex items-center gap-1">
            <CollapsibleTrigger asChild><Button type="button" variant="ghost" size="icon" aria-label={t("Query settings")}><SlidersHorizontal /></Button></CollapsibleTrigger>
            <Button size="icon" disabled={query.length === 0 || submitting || customResponseError !== null} type="submit" aria-label={submitting ? t("Submitting {{method}} Query", { method: modeLabel }) : t("Run {{method}} Query", { method: modeLabel })}>{submitting ? <LoaderCircle className="animate-spin" /> : <Play />}</Button>
          </div>
        </div>
      </Collapsible>
    </form>
  )
}

function queryModeLabelKey(mode: StudioQueryMode): string {
  if (mode === "basic") return "Basic"
  if (mode === "local") return "Local"
  if (mode === "global") return "Global"
  if (mode === "dynamic-global") return "Dynamic Global"
  return "DRIFT"
}

function explainabilityLabelKey(mode: ContentMode): string {
  if (mode === "metadata") return "Standard"
  if (mode === "content") return "Detailed"
  return "Debug"
}

function explainabilityDescriptionKey(mode: ContentMode): string {
  if (mode === "metadata") return "Records identifiers, scores, token usage and timing without storing full query, Context, Prompt or model content."
  if (mode === "content") return "Also records permitted full query, Context, Prompt and model response content for inspecting how the answer was produced."
  return "Records the most verbose supported diagnostic information for development and troubleshooting."
}

function responseStyleLabelKey(style: ResponseStyle): string {
  if (style === "standard") return "Standard"
  if (style === "concise") return "Concise"
  if (style === "detailed") return "Detailed"
  return "Custom"
}

function responseStyleDescriptionKey(style: ResponseStyle): string {
  if (style === "standard") return "A balanced answer using multiple paragraphs."
  if (style === "concise") return "A shorter answer in a single paragraph."
  if (style === "detailed") return "A more detailed answer organized into sections and paragraphs."
  return "Use your own response-length and formatting instruction."
}
