import { useState } from "react"
import { ChevronDown, History, MessageSquareText } from "lucide-react"

import { Button } from "@/components/ui/button"

interface QueryWorkspaceProps {
  composer: React.ReactNode
  answer: React.ReactNode
  trace: React.ReactNode
  runs: React.ReactNode
}

export function QueryWorkspace(props: QueryWorkspaceProps): React.ReactElement {
  const [answerOpen, setAnswerOpen] = useState(true)
  const [runsOpen, setRunsOpen] = useState(false)

  return (
    <section className="flex size-full min-h-0 flex-col bg-card/20" aria-label="Query workspace">
      <div className="shrink-0 border-b p-3">{props.composer}</div>
      <WorkspaceSection
        title="Answer"
        icon={<MessageSquareText className="size-4" />}
        open={answerOpen}
        onToggle={() => setAnswerOpen((value) => !value)}
        className={answerOpen ? "max-h-[38%] min-h-40" : ""}
      >
        {props.answer}
      </WorkspaceSection>
      <div className="min-h-48 flex-1 border-b">{props.trace}</div>
      <WorkspaceSection
        title="Previous Runs"
        icon={<History className="size-4" />}
        open={runsOpen}
        onToggle={() => setRunsOpen((value) => !value)}
        className={runsOpen ? "h-[38%] min-h-52" : ""}
      >
        {props.runs}
      </WorkspaceSection>
    </section>
  )
}

function WorkspaceSection(props: {
  title: string
  icon: React.ReactNode
  open: boolean
  onToggle: () => void
  className?: string
  children: React.ReactNode
}): React.ReactElement {
  return (
    <section className={props.className}>
      <Button
        variant="ghost"
        className="h-10 w-full justify-between rounded-none px-3"
        aria-expanded={props.open}
        onClick={props.onToggle}
      >
        <span className="flex items-center gap-2 text-xs font-semibold">{props.icon}{props.title}</span>
        <ChevronDown className={`size-4 transition-transform ${props.open ? "rotate-180" : ""}`} />
      </Button>
      <div className={props.open ? "min-h-0 h-[calc(100%-2.5rem)] overflow-hidden" : "hidden"}>{props.children}</div>
    </section>
  )
}
