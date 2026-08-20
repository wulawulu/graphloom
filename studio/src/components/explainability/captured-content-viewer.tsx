import { useState } from "react"
import { Copy } from "lucide-react"

import { SafeMarkdown } from "@/components/content/safe-markdown"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"

interface CapturedContentViewerProps {
  buttonLabel: string
  title: string
  content: string | null
  unavailableMessage: string
  testId: string
  preview?: boolean
  exactTabLabel?: string
  copyLabel?: string
  description?: string
}

export function CapturedContentViewer({
  buttonLabel,
  title,
  content,
  unavailableMessage,
  testId,
  preview = true,
  exactTabLabel = "Exact",
  copyLabel,
  description,
}: CapturedContentViewerProps): React.ReactElement {
  const [view, setView] = useState<"exact" | "preview">("exact")
  const [copied, setCopied] = useState(false)
  const copy = (): void => {
    if (content === null || navigator.clipboard === undefined) return
    void navigator.clipboard.writeText(content).then(() => setCopied(true)).catch(() => setCopied(false))
  }
  return (
    <Collapsible>
      <CollapsibleTrigger asChild>
        <Button variant="outline" size="sm" className="mt-1" aria-label={buttonLabel}>{buttonLabel}</Button>
      </CollapsibleTrigger>
      <CollapsibleContent className="mt-2 min-w-0 overflow-hidden rounded-md border bg-background/60 p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="min-w-0">
            <p className="truncate text-xs font-semibold">{title}</p>
            <p className="text-[10px] text-muted-foreground">{description ?? `Exact captured content is the source of truth${preview ? "; Preview is presentation only" : ""}.`}</p>
          </div>
          {content === null ? null : <Button variant="ghost" size="sm" onClick={copy} aria-label={copyLabel ?? `Copy exact ${title}`}><Copy className="size-3.5" />{copied ? "Copied" : "Copy"}</Button>}
        </div>
        {content === null ? (
          <p className="mt-3 rounded border bg-muted/20 p-3 text-xs text-muted-foreground">{unavailableMessage}</p>
        ) : (
          <div className="mt-3 min-w-0">
            {preview ? <div className="flex gap-1" role="tablist" aria-label={`${title} view`}><Button role="tab" aria-selected={view === "exact"} variant={view === "exact" ? "secondary" : "ghost"} size="sm" onClick={() => setView("exact")}>{exactTabLabel}</Button><Button role="tab" aria-selected={view === "preview"} variant={view === "preview" ? "secondary" : "ghost"} size="sm" onClick={() => setView("preview")}>Preview</Button></div> : null}
            {!preview || view === "exact"
              ? <pre className="mt-2 max-h-80 max-w-full overflow-auto whitespace-pre font-mono text-[11px] leading-5" data-testid={testId}>{content}</pre>
              : <SafeMarkdown className="markdown-answer mt-2 max-h-80 overflow-auto">{content}</SafeMarkdown>}
          </div>
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}
