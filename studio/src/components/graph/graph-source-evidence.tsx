import { useEffect, useRef, useState } from "react"
import { Check, Copy, FileText } from "lucide-react"
import { useTranslation } from "react-i18next"

import { getTextUnit } from "@/api/client"
import type { GraphTextUnitDetail, GraphTextUnitRef } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"

const INITIAL_SOURCE_COUNT = 5

interface GraphSourceEvidenceProps {
  sourceIds: string[]
  sources: GraphTextUnitRef[]
}

type SourceItem = { id: string; reference: GraphTextUnitRef | null }

function sourceItems(sourceIds: string[], sources: GraphTextUnitRef[]): SourceItem[] {
  const references = new Map(sources.map((source) => [source.id, source]))
  const seen = new Set<string>()
  return sourceIds
    .filter((id) => !seen.has(id) && seen.add(id))
    .map((id) => ({ id, reference: references.get(id) ?? null }))
}

function shortStableId(id: string): string {
  if (id.length <= 16) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}

function isAbort(reason: unknown): boolean {
  return reason instanceof DOMException && reason.name === "AbortError"
}

export function GraphSourceEvidence({ sourceIds, sources }: GraphSourceEvidenceProps): React.ReactElement {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const [selected, setSelected] = useState<GraphTextUnitRef | null>(null)
  const [detail, setDetail] = useState<GraphTextUnitDetail | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(false)
  const [copied, setCopied] = useState(false)
  const request = useRef<AbortController | null>(null)
  const items = sourceItems(sourceIds, sources)
  const visible = showAll ? items : items.slice(0, INITIAL_SOURCE_COUNT)

  useEffect(() => () => request.current?.abort(), [])

  const close = (): void => {
    request.current?.abort()
    request.current = null
    setSelected(null)
    setDetail(null)
    setLoading(false)
    setError(false)
    setCopied(false)
  }

  const open = (source: GraphTextUnitRef): void => {
    request.current?.abort()
    const controller = new AbortController()
    request.current = controller
    setSelected(source)
    setDetail(null)
    setLoading(true)
    setError(false)
    setCopied(false)
    void getTextUnit(source.id, controller.signal)
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
  }

  const copyId = (): void => {
    if (detail === null || navigator.clipboard === undefined) return
    void navigator.clipboard.writeText(detail.id).then(() => setCopied(true)).catch(() => setCopied(false))
  }

  return (
    <section className="space-y-2" aria-label={t("graph.sources.sourceEvidenceCount", { count: items.length })}>
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{t("graph.sources.sourceEvidenceCount", { count: items.length })}</h3>
      <div className="space-y-2">
        {visible.map((item) => item.reference === null
          ? <UnavailableSource key={item.id} id={item.id} />
          : <SourceCard key={item.id} source={item.reference} onOpen={open} />)}
        {items.length === 0 ? <p className="text-sm text-muted-foreground">{t("graph.sources.noSourceEvidence")}</p> : null}
        {items.length > INITIAL_SOURCE_COUNT ? <Button variant="ghost" size="sm" onClick={() => setShowAll((current) => !current)}>{t(showAll ? "graph.sources.showFewer" : "graph.sources.showAll")}</Button> : null}
      </div>

      <Sheet open={selected !== null} onOpenChange={(openState) => { if (!openState) close() }}>
        <SheetContent className="min-w-0">
          <SheetHeader>
            <SheetTitle>{selected === null ? t("graph.sources.sourceEvidence") : t("graph.sources.textUnitNumber", { shortId: selected.short_id })}</SheetTitle>
            <SheetDescription>{selected?.n_tokens === null || selected?.n_tokens === undefined ? t("graph.sources.exactSourceText") : t("graph.sources.tokenCount", { count: selected.n_tokens })}</SheetDescription>
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
    </section>
  )
}

function SourceCard({ onOpen, source }: { onOpen: (source: GraphTextUnitRef) => void; source: GraphTextUnitRef }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <Button type="button" variant="outline" className="h-auto w-full min-w-0 items-start justify-start gap-3 px-3 py-3 text-left" aria-label={t("graph.sources.viewTextUnit", { shortId: source.short_id })} onClick={() => onOpen(source)}>
      <span className="shrink-0 rounded-full bg-primary/10 p-2 text-primary"><FileText className="size-4" /></span>
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 flex-wrap items-center justify-between gap-1"><span className="font-medium">{t("graph.sources.textUnitNumber", { shortId: source.short_id })}</span>{source.n_tokens === null ? null : <Badge variant="outline">{t("graph.sources.tokenCount", { count: source.n_tokens })}</Badge>}</span>
        <span className="mt-1 line-clamp-3 block whitespace-normal break-words text-xs leading-5 font-normal text-muted-foreground">{source.preview}</span>
        <span className="mt-2 block text-[11px] font-medium text-primary">{t("graph.sources.viewSource")}</span>
      </span>
    </Button>
  )
}

function UnavailableSource({ id }: { id: string }): React.ReactElement {
  const { t } = useTranslation()
  return (
    <div className="min-w-0 rounded-md border border-dashed px-3 py-2.5" aria-disabled="true">
      <p className="text-sm font-medium">{t("graph.sources.sourceUnavailable")}</p>
      <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">{shortStableId(id)}</p>
    </div>
  )
}
