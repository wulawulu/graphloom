import type { ExplainabilityEventPayload } from "@/api/types"

export function DeveloperData({ event }: { event: ExplainabilityEventPayload }): React.ReactElement {
  return (
    <details className="rounded-md border bg-muted/20 p-3">
      <summary className="cursor-pointer text-xs font-medium text-muted-foreground">Developer data</summary>
      <p className="mt-2 font-mono text-[10px] text-muted-foreground">Raw JSON</p>
      <pre className="mt-1 max-h-72 overflow-auto rounded-md bg-muted p-3 font-mono text-[11px] leading-5">{JSON.stringify(event, null, 2)}</pre>
    </details>
  )
}
