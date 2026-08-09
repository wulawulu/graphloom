import { AlertCircle, Clock3, MessageSquareText, Sparkles } from "lucide-react"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"

import type { QueryResultState } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Skeleton } from "@/components/ui/skeleton"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"

interface AnswerPanelProps { runId: string | null; result: QueryResultState; loading: boolean }

export function AnswerPanel({ runId, result, loading }: AnswerPanelProps): React.ReactElement {
  return (
    <section className="flex size-full min-h-0 flex-col" aria-label="Final answer">
      <header className="flex h-11 shrink-0 items-center gap-2 border-b px-4"><MessageSquareText className="size-4 text-primary" /><h2 className="text-sm font-semibold">Final answer</h2></header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="p-4">
          {loading ? <><Skeleton className="mb-3 h-4 w-1/3" /><Skeleton className="h-20" /></> : null}
          {!loading && runId === null ? <AnswerState title="No result yet" detail="Select or submit a Query Run." /> : null}
          {!loading && result.state === "waiting" && runId !== null ? <AnswerState title="Query is running" detail="The final business result will appear here when the Run completes." /> : null}
          {!loading && result.state === "failed" ? <AnswerState title="Query did not complete" detail="Inspect the Explainability timeline for the safe failure summary." tone="error" /> : null}
          {!loading && result.state === "gone" ? <AnswerState title="Result no longer retained" detail="This run completed, but its process-local result is no longer available. Explainability history is still available." /> : null}
          {!loading && result.state === "missing" ? <AnswerState title="Result unavailable" detail="The Query Run does not exist in this Store namespace." /> : null}
          {!loading && result.state === "ready" ? (
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_18rem]">
              <article className="markdown-answer min-w-0"><ReactMarkdown remarkPlugins={[remarkGfm]} components={{ img: ({ alt }) => <span>[Remote image omitted{alt === undefined ? "" : `: ${alt}`}]</span>, a: ({ children, href }) => <a href={href} target="_blank" rel="noreferrer noopener">{children}</a> }}>{result.result.response}</ReactMarkdown></article>
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
      </ScrollArea>
    </section>
  )
}

function AnswerState({ title, detail, tone = "neutral" }: { title: string; detail: string; tone?: "neutral" | "error" }): React.ReactElement {
  return <div className="flex min-h-36 flex-col items-center justify-center text-center">{tone === "error" ? <AlertCircle className="mb-2 size-7 text-destructive" /> : <MessageSquareText className="mb-2 size-7 text-muted-foreground/50" />}<p className="text-sm font-medium">{title}</p><p className="mt-1 max-w-xl text-xs text-muted-foreground">{detail}</p></div>
}
