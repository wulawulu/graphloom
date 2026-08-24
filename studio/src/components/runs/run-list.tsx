import type { VariantProps } from "class-variance-authority"
import type { TFunction } from "i18next"
import { Clock3, RefreshCw } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { ExplainabilityRun } from "@/api/types"
import { Badge, type badgeVariants } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Skeleton } from "@/components/ui/skeleton"
import { activeStudioLocale } from "@/i18n"
import type { StudioTranslationKey } from "@/i18n/types"
import { cn } from "@/lib/utils"

interface RunListProps {
  runs: ExplainabilityRun[]
  selectedRunId: string | null
  loading: boolean
  error: StudioTranslationKey | null
  hasMore: boolean
  onSelect: (runId: string) => void
  onRefresh: () => void
  onLoadMore: () => void
}

function statusVariant(status: string): VariantProps<typeof badgeVariants>["variant"] {
  if (status === "completed") return "success"
  if (status === "failed" || status === "cancelled") return "destructive"
  if (status === "running" || status === "pending") return "warning"
  return "outline"
}

export function RunList(props: RunListProps): React.ReactElement {
  const { t } = useTranslation()
  return (
    <section className="flex min-h-0 flex-1 flex-col" aria-label={t("answer.labels.queryRunHistory")}>
      <div className="flex items-center justify-between py-2">
        <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{t("runs.actions.runHistory")}</h2>
        <Button variant="ghost" size="icon" onClick={props.onRefresh} aria-label={t("runs.actions.refreshRunHistory")}>
          <RefreshCw className="size-3.5" />
        </Button>
      </div>
      {props.error !== null ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-xs text-red-300">{t(props.error)}</p> : null}
      <ScrollArea className="min-h-0 flex-1">
        <div className="space-y-1 pr-2">
          {props.runs.map((run) => (
            <button
              key={run.run_id}
              type="button"
              onClick={() => props.onSelect(run.run_id)}
              className={cn(
                "w-full rounded-md border p-2.5 text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                props.selectedRunId === run.run_id && "border-primary/50 bg-primary/5",
              )}
            >
              <div className="mb-1.5 flex items-center justify-between gap-2">
                <div className="flex min-w-0 gap-1"><Badge variant={statusVariant(run.status)}>{statusLabel(t, run.status)}</Badge><Badge variant="outline" className="max-w-32 truncate">{queryMethodLabel(t, run.query_method)}</Badge></div>
                <span className="shrink-0 font-mono text-[10px] text-muted-foreground">{t("runs.counts.countEvent", { count: run.event_count })}</span>
              </div>
              <p className="line-clamp-2 text-xs leading-5">{run.query ?? t("query.labels.queryHiddenMetadataMode")}</p>
              <p className="mt-1.5 flex items-center gap-1 text-[10px] text-muted-foreground">
                <Clock3 className="size-3" /> {new Date(run.started_at).toLocaleString(activeStudioLocale())}
              </p>
            </button>
          ))}
          {props.loading ? <><Skeleton className="h-20" /><Skeleton className="h-20" /></> : null}
          {!props.loading && props.runs.length === 0 ? <p className="py-8 text-center text-xs text-muted-foreground">{t("runs.messages.noRunsYetSubmitALocalQueryToBegin")}</p> : null}
          {props.hasMore ? <Button variant="outline" size="sm" className="w-full" disabled={props.loading} onClick={props.onLoadMore}>{t("runs.actions.loadMore")}</Button> : null}
        </div>
      </ScrollArea>
    </section>
  )
}

function statusLabel(t: TFunction, status: string): string {
  const keys: Readonly<Record<string, StudioTranslationKey>> = {
    completed: "answer.status.completed",
    failed: "answer.status.failed",
    cancelled: "answer.status.cancelled",
    running: "answer.status.running",
    pending: "answer.status.pending",
  }
  const key = keys[status]
  return key === undefined ? status : t(key)
}

function queryMethodLabel(t: TFunction, method: string | undefined): string {
  if (method === "basic") return t("query.methods.basic")
  if (method === "local") return t("query.methods.local")
  if (method === "global") return t("query.methods.global")
  if (method === "drift") return t("query.methods.drift")
  return method ?? t("runs.status.unknownMethod")
}
