import { useState } from "react"
import { Check } from "lucide-react"

import type { ExplainabilityCandidate } from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"

const PAGE_SIZE = 100

interface CandidateTableProps {
  candidates: ExplainabilityCandidate[]
  label?: string
}

export function CandidateTable({ candidates, label = "Candidates" }: CandidateTableProps): React.ReactElement {
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE)
  const visible = candidates.slice(0, visibleCount)
  const selectedCount = candidates.filter((candidate) => candidate.selected).length
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between text-xs"><span className="font-medium">{label}</span><span className="text-muted-foreground">Selected {selectedCount} / Candidates {candidates.length}</span></div>
      <div className="overflow-x-auto rounded-md border">
        <Table>
          <TableHeader><TableRow><TableHead>Rank</TableHead><TableHead>Title / ID</TableHead><TableHead>Type</TableHead><TableHead>Score</TableHead><TableHead>Selected</TableHead><TableHead>Reason</TableHead><TableHead>Depth</TableHead></TableRow></TableHeader>
          <TableBody>
            {visible.map((candidate) => (
              <TableRow key={`${candidate.record_type}:${candidate.id}`} className={candidate.selected ? "bg-primary/10" : "opacity-65"} data-selected={candidate.selected}>
                <TableCell>{candidate.rank ?? "—"}</TableCell>
                <TableCell><span className="block max-w-52 truncate font-medium">{candidate.title ?? candidate.id}</span>{candidate.title !== undefined ? <span className="block max-w-52 truncate font-mono text-[10px] text-muted-foreground">{candidate.id}</span> : null}{candidate.source_id !== undefined || candidate.relationship_id !== undefined ? <span className="block max-w-52 truncate text-[10px] text-muted-foreground">{candidate.source_id !== undefined ? `source ${candidate.source_id}` : ""}{candidate.relationship_id !== undefined ? ` via ${candidate.relationship_id}` : ""}</span> : null}</TableCell>
                <TableCell>{candidate.record_type}</TableCell>
                <TableCell>{candidate.score === undefined ? "—" : candidate.score.toFixed(4)}</TableCell>
                <TableCell>{candidate.selected ? <Check className="size-4 text-success" aria-label="Selected" /> : "—"}</TableCell>
                <TableCell>{candidate.reason === undefined ? "—" : <Badge variant="outline">{candidate.reason.replaceAll("_", " ")}</Badge>}</TableCell>
                <TableCell>{candidate.expansion_depth ?? "—"}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      {visible.length < candidates.length ? <div className="flex items-center justify-between"><span className="text-xs text-muted-foreground">Showing {visible.length} of {candidates.length}</span><Button variant="outline" size="sm" onClick={() => setVisibleCount((count) => Math.min(count + PAGE_SIZE, candidates.length))}>Show 100 more</Button></div> : null}
    </div>
  )
}
