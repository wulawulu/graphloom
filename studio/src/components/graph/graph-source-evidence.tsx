import { useState } from "react"
import { FileText } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { GraphTextUnitRef } from "@/api/types"
import { TextUnitDetailSheet, type TextUnitDetailReference } from "@/components/graph/text-unit-detail-sheet"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

const INITIAL_SOURCE_COUNT = 5
const NO_LOADING_SOURCES: ReadonlySet<string> = new Set()

interface GraphSourceEvidenceProps {
  sourceIds: string[]
  sources: GraphTextUnitRef[]
  loadingSourceIds?: ReadonlySet<string>
}

type SourceItem = { id: string; reference: GraphTextUnitRef | null; loading: boolean }

function sourceItems(sourceIds: string[], sources: GraphTextUnitRef[], loadingSourceIds: ReadonlySet<string>): SourceItem[] {
  const references = new Map(sources.map((source) => [source.id, source]))
  const seen = new Set<string>()
  return sourceIds
    .filter((id) => !seen.has(id) && seen.add(id))
    .map((id) => ({ id, reference: references.get(id) ?? null, loading: loadingSourceIds.has(id) }))
}

function shortStableId(id: string): string {
  if (id.length <= 16) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}

export function GraphSourceEvidence({ sourceIds, sources, loadingSourceIds = NO_LOADING_SOURCES }: GraphSourceEvidenceProps): React.ReactElement {
  const { t } = useTranslation()
  const [showAll, setShowAll] = useState(false)
  const [selected, setSelected] = useState<TextUnitDetailReference | null>(null)
  const items = sourceItems(sourceIds, sources, loadingSourceIds)
  const visible = showAll ? items : items.slice(0, INITIAL_SOURCE_COUNT)

  const open = (source: GraphTextUnitRef): void => {
    setSelected({ id: source.id, shortId: source.short_id, nTokens: source.n_tokens })
  }

  return (
    <section className="space-y-2" aria-label={t("graph.sources.sourceEvidenceCount", { count: items.length })}>
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{t("graph.sources.sourceEvidenceCount", { count: items.length })}</h3>
      <div className="space-y-2">
        {visible.map((item) => item.reference === null
          ? item.loading ? <LoadingSource key={item.id} /> : <UnavailableSource key={item.id} id={item.id} />
          : <SourceCard key={item.id} source={item.reference} onOpen={open} />)}
        {items.length === 0 ? <p className="text-sm text-muted-foreground">{t("graph.sources.noSourceEvidence")}</p> : null}
        {items.length > INITIAL_SOURCE_COUNT ? <Button variant="ghost" size="sm" onClick={() => setShowAll((current) => !current)}>{t(showAll ? "graph.sources.showFewer" : "graph.sources.showAll")}</Button> : null}
      </div>

      <TextUnitDetailSheet reference={selected} onClose={() => setSelected(null)} />
    </section>
  )
}

function LoadingSource(): React.ReactElement {
  const { t } = useTranslation()
  return <div className="min-w-0 rounded-md border border-dashed px-3 py-4 text-center text-sm text-muted-foreground" role="status">{t("graph.sources.loadingSource")}</div>
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
