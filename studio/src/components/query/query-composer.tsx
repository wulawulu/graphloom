import { useEffect, useState, type SyntheticEvent } from "react"
import { LoaderCircle, Play, SlidersHorizontal } from "lucide-react"
import { toast } from "sonner"

import { ApiError, startQuery } from "@/api/client"
import type { ContentMode, StartQueryMethod, StartQueryResponse } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"

interface QueryComposerProps {
  onAccepted: (response: StartQueryResponse, submittedQuery: string) => void
  resetRevision: number
}

const modeHelp: Record<ContentMode, string> = {
  metadata: "Hides full query, context, and prompt content from explainability.",
  content: "Includes permitted query, context, and model content.",
  debug: "Uses the most verbose supported explainability mode.",
}

type StudioQueryMode = "basic" | "local" | "global" | "dynamic-global" | "drift"

interface QueryModeMetadata {
  label: string
  method: StartQueryMethod
  dynamicCommunitySelection: boolean
}

const queryModes: Record<StudioQueryMode, QueryModeMetadata> = {
  basic: { label: "Basic", method: "basic", dynamicCommunitySelection: false },
  local: { label: "Local", method: "local", dynamicCommunitySelection: false },
  global: { label: "Global", method: "global", dynamicCommunitySelection: false },
  "dynamic-global": { label: "Dynamic Global", method: "global", dynamicCommunitySelection: true },
  drift: { label: "DRIFT", method: "drift", dynamicCommunitySelection: false },
}

const queryModeOptions = Object.entries(queryModes) as Array<[StudioQueryMode, QueryModeMetadata]>

export function QueryComposer({ onAccepted, resetRevision }: QueryComposerProps): React.ReactElement {
  const [query, setQuery] = useState("")
  const [queryMode, setQueryMode] = useState<StudioQueryMode>("local")
  const [contentMode, setContentMode] = useState<ContentMode>("metadata")
  const [responseType, setResponseType] = useState("Multiple Paragraphs")
  const [submitting, setSubmitting] = useState(false)

  useEffect(() => setQuery(""), [resetRevision])

  const submit = async (event: SyntheticEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    if (query.length === 0 || submitting) return
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
      toast.success(`${mode.label} Query accepted`)
    } catch (error) {
      const message = error instanceof ApiError && error.status === 429
        ? "Studio is at its active Query limit. Try again after a Run finishes."
        : "The Query could not be accepted."
      toast.error(message)
    } finally {
      setSubmitting(false)
    }
  }

  const mode = queryModes[queryMode]

  return (
    <form className="rounded-lg border bg-card shadow-sm" onSubmit={(event) => void submit(event)}>
      <div className="p-2 pb-0">
        <label htmlFor="studio-query" className="sr-only">Ask about the graph</label>
        <Textarea
          id="studio-query"
          className="min-h-20 resize-none border-0 bg-transparent px-1 shadow-none focus-visible:ring-0"
          value={query}
          maxLength={1024 * 1024}
          placeholder="Ask a question about the indexed graph…"
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      <Collapsible>
        <CollapsibleContent className="space-y-3 border-t px-3 py-3">
          <div className="space-y-1.5">
            <label htmlFor="content-mode" className="text-xs font-medium text-muted-foreground">Explainability content</label>
            <Select value={contentMode} onValueChange={(value) => setContentMode(value as ContentMode)}>
              <SelectTrigger id="content-mode"><SelectValue /></SelectTrigger>
              <SelectContent><SelectItem value="metadata">Metadata</SelectItem><SelectItem value="content">Content</SelectItem><SelectItem value="debug">Debug</SelectItem></SelectContent>
            </Select>
            <p className="text-[11px] leading-4 text-muted-foreground">{modeHelp[contentMode]}</p>
          </div>
          <div className="space-y-1.5">
            <label htmlFor="response-type" className="text-xs font-medium text-muted-foreground">Response type</label>
            <Input id="response-type" value={responseType} maxLength={256} onChange={(event) => setResponseType(event.target.value)} />
          </div>
        </CollapsibleContent>
        <div className="flex items-center justify-between border-t px-2 py-1.5">
          <div className="flex items-center gap-1">
            <Select disabled={submitting} value={queryMode} onValueChange={(value) => setQueryMode(value as StudioQueryMode)}>
              <SelectTrigger aria-label="Query method" className="h-6 w-auto gap-1 px-2 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {queryModeOptions.map(([value, metadata]) => (
                  <SelectItem key={value} value={value}>{metadata.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Badge variant="outline">{contentMode[0]?.toUpperCase()}{contentMode.slice(1)}</Badge>
          </div>
          <div className="flex items-center gap-1">
            <CollapsibleTrigger asChild><Button type="button" variant="ghost" size="icon" aria-label="Query settings"><SlidersHorizontal /></Button></CollapsibleTrigger>
            <Button size="icon" disabled={query.length === 0 || submitting} type="submit" aria-label={submitting ? `Submitting ${mode.label} Query` : `Run ${mode.label} Query`}>{submitting ? <LoaderCircle className="animate-spin" /> : <Play />}</Button>
          </div>
        </div>
      </Collapsible>
    </form>
  )
}
