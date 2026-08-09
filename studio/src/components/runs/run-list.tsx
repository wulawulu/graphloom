import { Clock3, RefreshCw } from "lucide-react"

import type { ExplainabilityRun } from "@/api/types"
import { Badge, type badgeVariants } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"
import type { VariantProps } from "class-variance-authority"

interface RunListProps {
  runs: ExplainabilityRun[]
  selectedRunId: string | null
  loading: boolean
  error: string | null
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
  return (
    <section className="flex min-h-0 flex-1 flex-col" aria-label="Query run history">
      <div className="flex items-center justify-between py-2">
        <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">Run history</h2>
        <Button variant="ghost" size="icon" onClick={props.onRefresh} aria-label="Refresh run history">
          <RefreshCw className="size-3.5" />
        </Button>
      </div>
      {props.error !== null ? <p className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-xs text-red-300">{props.error}</p> : null}
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
                <div className="flex gap-1"><Badge variant={statusVariant(run.status)}>{run.status}</Badge><Badge variant="outline">{run.query_method ?? "unknown method"}</Badge></div>
                <span className="font-mono text-[10px] text-muted-foreground">{run.event_count} events</span>
              </div>
              <p className="line-clamp-2 text-xs leading-5">{run.query ?? "Query hidden (metadata mode)"}</p>
              <p className="mt-1.5 flex items-center gap-1 text-[10px] text-muted-foreground">
                <Clock3 className="size-3" /> {new Date(run.started_at).toLocaleString()}
              </p>
            </button>
          ))}
          {props.loading ? <><Skeleton className="h-20" /><Skeleton className="h-20" /></> : null}
          {!props.loading && props.runs.length === 0 ? <p className="py-8 text-center text-xs text-muted-foreground">No runs yet. Submit a Local Query to begin.</p> : null}
          {props.hasMore ? <Button variant="outline" size="sm" className="w-full" disabled={props.loading} onClick={props.onLoadMore}>Load more</Button> : null}
        </div>
      </ScrollArea>
    </section>
  )
}
