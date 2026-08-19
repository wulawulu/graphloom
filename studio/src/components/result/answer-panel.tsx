import { useMemo } from "react"
import { AlertCircle, Clock3, MessageSquareText, Sparkles } from "lucide-react"

import type { ExplainabilityEnvelope, QueryResultState } from "@/api/types"
import { SafeMarkdown } from "@/components/content/safe-markdown"
import { Badge } from "@/components/ui/badge"
import { Skeleton } from "@/components/ui/skeleton"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { buildCitationGraphIndex, resolveCitationGroup, type CitationGroup, type GraphEmphasis } from "@/lib/citations"

interface AnswerPanelProps {
  runId: string | null
  result: QueryResultState
  loading: boolean
  envelopes?: ExplainabilityEnvelope[]
  onCitationEmphasis?: (emphasis: GraphEmphasis) => void
}

export function AnswerPanel({ runId, result, loading, envelopes = [], onCitationEmphasis }: AnswerPanelProps): React.ReactElement {
  const citationIndex = useMemo(() => buildCitationGraphIndex(envelopes), [envelopes])
  const renderCitation = (group: CitationGroup): React.ReactNode => {
    const emphasis = resolveCitationGroup(group, citationIndex)
    const title = `${group.dataset}\n${group.recordIds.join("\n")}${group.hasMore ? "\n+more" : ""}`
    const label = `${group.dataset} · ${group.recordIds.length}${group.hasMore ? "+" : ""}`
    if (emphasis === null || onCitationEmphasis === undefined) {
      return <span className="mx-0.5 inline-flex items-center rounded-full border bg-muted/50 px-2 py-0.5 align-baseline text-[11px] font-medium text-muted-foreground" title={title}>{label}</span>
    }
    return <button type="button" className="mx-0.5 inline-flex items-center rounded-full border border-primary/40 bg-primary/10 px-2 py-0.5 align-baseline text-[11px] font-medium text-primary transition-colors hover:bg-primary/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={title} aria-label={`Emphasize ${group.recordIds.length} ${group.dataset} in graph`} onClick={() => onCitationEmphasis(emphasis)}>{label}</button>
  }

  return (
    <section aria-label="Final answer">
        <div className="px-3 pb-3">
          {loading ? <><Skeleton className="mb-3 h-4 w-1/3" /><Skeleton className="h-20" /></> : null}
          {!loading && runId === null ? <AnswerState title="No result yet" detail="Select or submit a Query Run." /> : null}
          {!loading && result.state === "waiting" && runId !== null ? <AnswerState title="Query is running" detail="The final business result will appear here when the Run completes." /> : null}
          {!loading && result.state === "failed" ? <AnswerState title="Query did not complete" detail="Inspect the Explainability timeline for the safe failure summary." tone="error" /> : null}
          {!loading && result.state === "gone" ? <AnswerState title="Result no longer retained" detail="This run completed, but its process-local result is no longer available. Explainability history is still available." /> : null}
          {!loading && result.state === "missing" ? <AnswerState title="Result unavailable" detail="The Query Run does not exist in this Store namespace." /> : null}
          {!loading && result.state === "ready" ? (
            <div className="grid gap-4">
              <article className="min-w-0"><SafeMarkdown renderCitation={renderCitation}>{result.result.response}</SafeMarkdown></article>
              <aside className="space-y-3">
                <div className="flex flex-wrap gap-2">
                  <Badge variant="outline"><Clock3 /> {result.result.elapsed_ms} ms</Badge>
                  <Badge variant="outline"><Sparkles /> {result.result.usage.llm_calls} calls</Badge>
                  <Badge variant="outline">{result.result.usage.prompt_tokens} input</Badge>
                  <Badge variant="outline">{result.result.usage.output_tokens} output</Badge>
                </div>
                {Object.keys(result.result.usage.categories).length > 0 ? (
                  <Table>
                    <TableHeader><TableRow><TableHead>Category</TableHead><TableHead>Calls</TableHead><TableHead>Tokens</TableHead></TableRow></TableHeader>
                    <TableBody>{Object.entries(result.result.usage.categories).map(([name, usage]) => <TableRow key={name}><TableCell>{name}</TableCell><TableCell>{usage.llm_calls}</TableCell><TableCell>{usage.prompt_tokens}/{usage.output_tokens}</TableCell></TableRow>)}</TableBody>
                  </Table>
                ) : null}
              </aside>
            </div>
          ) : null}
        </div>
    </section>
  )
}

function AnswerState({ title, detail, tone = "neutral" }: { title: string; detail: string; tone?: "neutral" | "error" }): React.ReactElement {
  return <div className="flex flex-col items-start py-3">{tone === "error" ? <AlertCircle className="mb-2 size-5 text-destructive" /> : <MessageSquareText className="mb-2 size-5 text-muted-foreground/50" />}<p className="text-sm font-medium">{title}</p><p className="mt-1 max-w-xl text-xs text-muted-foreground">{detail}</p></div>
}
