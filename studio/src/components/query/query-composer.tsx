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
import type { StudioTranslationKey } from "@/i18n/types"

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
      toast.success(t("query.toast.accepted", { method: t(queryModeLabelKey(queryMode)) }))
    } catch (error) {
      const message = error instanceof ApiError && error.status === 429
        ? t("query.messages.studioIsAtItsActiveQueryLimitTryAgainAfterARunFinishes")
        : t("query.messages.theQueryCouldNotBeAccepted")
      toast.error(message)
    } finally {
      setSubmitting(false)
    }
  }

  const modeLabel = t(queryModeLabelKey(queryMode))
  const responseType = responseTypeForStyle(responseStyle, customResponse)
  const customResponseError = responseStyle === "custom" && responseType.length === 0
    ? t("query.responseStyle.custom.required")
    : responseStyle === "custom" && utf8ByteLength(responseType) > MAX_RESPONSE_TYPE_BYTES
      ? t("query.responseStyle.custom.byteLimit", { count: MAX_RESPONSE_TYPE_BYTES })
      : null

  return (
    <form className="rounded-lg border bg-card shadow-sm" onSubmit={(event) => void submit(event)}>
      <div className="p-2 pb-0">
        <label htmlFor="studio-query" className="sr-only">{t("query.composer.label")}</label>
        <Textarea
          id="studio-query"
          className="min-h-20 resize-none border-0 bg-transparent px-1 shadow-none focus-visible:ring-0"
          value={query}
          maxLength={1024 * 1024}
          placeholder={t("query.composer.placeholder")}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      <Collapsible>
        <CollapsibleContent className="space-y-3 border-t px-3 py-3">
          <div className="space-y-1.5">
            <label htmlFor="content-mode" className="text-xs font-medium text-muted-foreground">{t("query.explainability.title")}</label>
            <Select value={contentMode} onValueChange={(value) => setContentMode(value as ContentMode)}>
              <SelectTrigger id="content-mode"><SelectValue /></SelectTrigger>
              <SelectContent>{explainabilityDetailOptions.map((value) => <SelectItem key={value} value={value}>{t(explainabilityLabelKey(value))}</SelectItem>)}</SelectContent>
            </Select>
            <p className="text-[11px] leading-4 text-muted-foreground">{t(explainabilityDescriptionKey(contentMode))}</p>
          </div>
          <div className="space-y-1.5">
            <label htmlFor="response-style" className="text-xs font-medium text-muted-foreground">{t("query.responseStyle.title")}</label>
            <Select value={responseStyle} onValueChange={(value) => setResponseStyle(value as ResponseStyle)}>
              <SelectTrigger id="response-style"><SelectValue /></SelectTrigger>
              <SelectContent>{responseStyleOptions.map(([value]) => <SelectItem key={value} value={value}>{t(responseStyleLabelKey(value))}</SelectItem>)}</SelectContent>
            </Select>
            <p className="text-[11px] leading-4 text-muted-foreground">{t(responseStyleDescriptionKey(responseStyle))}</p>
            {responseStyle === "custom" ? (
              <div className="space-y-1.5 pt-1">
                <label htmlFor="custom-response" className="text-xs font-medium text-muted-foreground">{t("query.responseStyle.custom.instructionsLabel")}</label>
                <Input
                  id="custom-response"
                  value={customResponse}
                  aria-invalid={customResponseError !== null}
                  aria-describedby={customResponseError === null ? undefined : "custom-response-error"}
                  placeholder={t("query.responseStyle.custom.placeholder")}
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
              <SelectTrigger aria-label={t("query.method.label")} className="h-6 w-auto gap-1 px-2 text-xs">
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
            <CollapsibleTrigger asChild><Button type="button" variant="ghost" size="icon" aria-label={t("query.settings.title")}><SlidersHorizontal /></Button></CollapsibleTrigger>
            <Button size="icon" disabled={query.length === 0 || submitting || customResponseError !== null} type="submit" aria-label={submitting ? t("query.actions.submitting", { method: modeLabel }) : t("query.actions.run", { method: modeLabel })}>{submitting ? <LoaderCircle className="animate-spin" /> : <Play />}</Button>
          </div>
        </div>
      </Collapsible>
    </form>
  )
}

function queryModeLabelKey(mode: StudioQueryMode): StudioTranslationKey {
  if (mode === "basic") return "query.methods.basic"
  if (mode === "local") return "query.methods.local"
  if (mode === "global") return "query.methods.global"
  if (mode === "dynamic-global") return "query.methods.dynamicGlobal"
  return "query.methods.drift"
}

function explainabilityLabelKey(mode: ContentMode): StudioTranslationKey {
  if (mode === "metadata") return "query.explainability.standard.label"
  if (mode === "content") return "query.explainability.detailed.label"
  return "query.explainability.debug.label"
}

function explainabilityDescriptionKey(mode: ContentMode): StudioTranslationKey {
  if (mode === "metadata") return "query.explainability.standard.description"
  if (mode === "content") return "query.explainability.detailed.description"
  return "query.explainability.debug.description"
}

function responseStyleLabelKey(style: ResponseStyle): StudioTranslationKey {
  if (style === "standard") return "query.responseStyle.standard.label"
  if (style === "concise") return "query.responseStyle.concise.label"
  if (style === "detailed") return "query.responseStyle.detailed.label"
  return "query.responseStyle.custom.label"
}

function responseStyleDescriptionKey(style: ResponseStyle): StudioTranslationKey {
  if (style === "standard") return "query.responseStyle.standard.description"
  if (style === "concise") return "query.responseStyle.concise.description"
  if (style === "detailed") return "query.responseStyle.detailed.description"
  return "query.responseStyle.custom.description"
}
