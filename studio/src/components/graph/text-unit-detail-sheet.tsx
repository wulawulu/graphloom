import { useEffect, useRef, useState } from "react"
import { Check, Copy } from "lucide-react"
import { useTranslation } from "react-i18next"

import { getTextUnit } from "@/api/client"
import type { GraphTextUnitDetail } from "@/api/types"
import { Button } from "@/components/ui/button"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"

export interface TextUnitDetailReference {
  id: string
  shortId?: string
  nTokens?: number | null
}

interface TextUnitDetailSheetProps {
  reference: TextUnitDetailReference | null
  onClose: () => void
}

function isAbort(reason: unknown): boolean {
  return reason instanceof DOMException && reason.name === "AbortError"
}

export function TextUnitDetailSheet({ reference, onClose }: TextUnitDetailSheetProps): React.ReactElement {
  const { t } = useTranslation()
  const [detail, setDetail] = useState<GraphTextUnitDetail | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(false)
  const [copied, setCopied] = useState(false)
  const request = useRef<AbortController | null>(null)

  useEffect(() => {
    request.current?.abort()
    setDetail(null)
    setError(false)
    setCopied(false)
    if (reference === null) {
      setLoading(false)
      request.current = null
      return
    }
    const controller = new AbortController()
    request.current = controller
    setLoading(true)
    void getTextUnit(reference.id, controller.signal)
      .then((value) => {
        if (request.current === controller && !controller.signal.aborted) setDetail(value)
      })
      .catch((reason: unknown) => {
        if (request.current === controller && !isAbort(reason)) setError(true)
      })
      .finally(() => {
        if (request.current === controller) {
          request.current = null
          setLoading(false)
        }
      })
    return () => controller.abort()
  }, [reference])

  const copyId = (): void => {
    if (detail === null || navigator.clipboard === undefined) return
    void navigator.clipboard.writeText(detail.id).then(() => setCopied(true)).catch(() => setCopied(false))
  }

  const shortId = detail?.short_id ?? reference?.shortId
  const nTokens = detail?.n_tokens ?? reference?.nTokens
  return (
    <Sheet open={reference !== null} onOpenChange={(open) => { if (!open) onClose() }}>
      <SheetContent className="min-w-0">
        <SheetHeader>
          <SheetTitle>{shortId === undefined ? t("graph.sources.sourceEvidence") : t("graph.sources.textUnitNumber", { shortId })}</SheetTitle>
          <SheetDescription>{nTokens === null || nTokens === undefined ? t("graph.sources.exactSourceText") : t("graph.sources.tokenCount", { count: nTokens })}</SheetDescription>
        </SheetHeader>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {loading ? <p className="py-12 text-center text-sm text-muted-foreground">{t("graph.sources.loadingSource")}</p> : null}
          {error ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-red-300">{t("graph.sources.sourceUnavailable")}</p> : null}
          {detail === null ? null : (
            <div className="space-y-4">
              <pre className="max-w-full whitespace-pre-wrap break-words rounded-md border bg-muted/20 p-4 font-sans text-sm leading-6" data-testid="text-unit-exact-text">{detail.text}</pre>
              <details className="rounded-md border bg-muted/20 p-3 text-xs">
                <summary className="cursor-pointer font-medium text-muted-foreground">{t("graph.labels.metadata")}</summary>
                <dl className="mt-3 space-y-3">
                  <div><dt className="text-muted-foreground">{t("graph.labels.shortId")}</dt><dd className="break-all">{detail.short_id}</dd></div>
                  {detail.document_id === null ? null : <div><dt className="text-muted-foreground">{t("graph.sources.documentId")}</dt><dd className="break-all">{detail.document_id}</dd></div>}
                  <div><dt className="text-muted-foreground">ID</dt><dd className="mt-1 flex min-w-0 items-start gap-2"><code className="min-w-0 flex-1 break-all">{detail.id}</code><Button variant="ghost" size="icon" className="shrink-0" aria-label={t("graph.actions.copyId")} onClick={copyId}>{copied ? <Check /> : <Copy />}</Button></dd></div>
                </dl>
              </details>
            </div>
          )}
        </div>
      </SheetContent>
    </Sheet>
  )
}
