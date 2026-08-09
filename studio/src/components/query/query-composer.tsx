import { useState, type SyntheticEvent } from "react"
import { ChevronDown, LoaderCircle, Play } from "lucide-react"
import { toast } from "sonner"

import { ApiError, startQuery } from "@/api/client"
import type { ContentMode, StartQueryResponse } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"

interface QueryComposerProps {
  onAccepted: (response: StartQueryResponse) => void
}

const modeHelp: Record<ContentMode, string> = {
  metadata: "Hides full query, context, and prompt content from explainability.",
  content: "Includes permitted query, context, and model content.",
  debug: "Uses the most verbose supported explainability mode.",
}

export function QueryComposer({ onAccepted }: QueryComposerProps): React.ReactElement {
  const [query, setQuery] = useState("")
  const [contentMode, setContentMode] = useState<ContentMode>("metadata")
  const [responseType, setResponseType] = useState("Multiple Paragraphs")
  const [submitting, setSubmitting] = useState(false)

  const submit = async (event: SyntheticEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    if (query.length === 0 || submitting) return
    setSubmitting(true)
    try {
      const response = await startQuery({
        query,
        method: "local",
        content_mode: contentMode,
        response_type: responseType,
      })
      onAccepted(response)
      toast.success("Local Query accepted")
    } catch (error) {
      const message = error instanceof ApiError && error.status === 429
        ? "Studio is at its active Query limit. Try again after a Run finishes."
        : "The Query could not be accepted."
      toast.error(message)
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <form className="space-y-3" onSubmit={(event) => void submit(event)}>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>Method</span><Badge variant="outline">Local</Badge>
      </div>
      <div className="space-y-1.5">
        <label htmlFor="studio-query" className="text-xs font-medium text-muted-foreground">New Local Query</label>
        <Textarea
          id="studio-query"
          value={query}
          maxLength={1024 * 1024}
          placeholder="Ask a question about the indexed graph…"
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      <div className="space-y-1.5">
        <label htmlFor="content-mode" className="text-xs font-medium text-muted-foreground">Explainability content</label>
        <Select value={contentMode} onValueChange={(value) => setContentMode(value as ContentMode)}>
          <SelectTrigger id="content-mode"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="metadata">Metadata</SelectItem>
            <SelectItem value="content">Content</SelectItem>
            <SelectItem value="debug">Debug</SelectItem>
          </SelectContent>
        </Select>
        <p className="text-[11px] leading-4 text-muted-foreground">{modeHelp[contentMode]}</p>
      </div>
      <Collapsible>
        <CollapsibleTrigger asChild>
          <Button type="button" variant="ghost" size="sm" className="w-full justify-between px-1">
            Advanced <ChevronDown className="size-3" />
          </Button>
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-2">
          <label htmlFor="response-type" className="mb-1.5 block text-xs font-medium text-muted-foreground">Response type</label>
          <Input id="response-type" value={responseType} maxLength={256} onChange={(event) => setResponseType(event.target.value)} />
        </CollapsibleContent>
      </Collapsible>
      <Button className="w-full" disabled={query.length === 0 || submitting} type="submit">
        {submitting ? <LoaderCircle className="animate-spin" /> : <Play />}
        {submitting ? "Submitting" : "Run Local Query"}
      </Button>
    </form>
  )
}
