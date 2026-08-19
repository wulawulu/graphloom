import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { SafeMarkdown } from "@/components/content/safe-markdown"
import { AnswerPanel } from "@/components/result/answer-panel"

afterEach(cleanup)

describe("AnswerPanel", () => {
  it.each([
    [{ state: "waiting" } as const, "Query is running"],
    [{ state: "failed" } as const, "Query did not complete"],
    [{ state: "gone" } as const, "Result no longer retained"],
  ])("renders lifecycle state", (result, label) => {
    render(<AnswerPanel runId="run" result={result} loading={false} />)
    expect(screen.getByText(label)).toBeInTheDocument()
  })

  it("renders markdown without raw HTML and shows usage", () => {
    render(<AnswerPanel runId="run" loading={false} result={{ state: "ready", result: { run_id: "run", response: "**Answer** <script>alert(1)</script>", elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }} />)
    expect(screen.getByText("Answer")).toBeInTheDocument()
    expect(document.querySelector("script")).toBeNull()
    expect(screen.getByText("1 calls")).toBeInTheDocument()
  })

  it("does not load remote images embedded in model Markdown", () => {
    render(<AnswerPanel runId="run" loading={false} result={{ state: "ready", result: { run_id: "run", response: "![tracker](https://example.invalid/track.png)", elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }} />)
    expect(screen.queryByRole("img", { name: "tracker" })).not.toBeInTheDocument()
    expect(screen.getByText("[Remote image omitted: tracker]")).toBeInTheDocument()
  })

  it("keeps ordinary external Markdown links isolated from citation rendering", () => {
    render(<AnswerPanel runId="run" loading={false} result={{ state: "ready", result: { run_id: "run", response: "Read [documentation](https://example.com).", elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }} />)
    expect(screen.getByRole("link", { name: "documentation" })).toHaveAttribute("target", "_blank")
    expect(screen.getByRole("link", { name: "documentation" })).toHaveAttribute("rel", "noreferrer noopener")
  })

  it("renders graph citations as safe chips and resolves selected stable IDs", async () => {
    const user = userEvent.setup()
    const onCitationEmphasis = vi.fn()
    render(<AnswerPanel
      runId="run"
      loading={false}
      result={{ state: "ready", result: { run_id: "run", response: "Evidence [Data: Entities (150, 0); Relationships (23); Reports (1)]", elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }}
      envelopes={[
        { schema_version: 1, sequence: 1, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "entities_selected", entities: [{ id: "entity-150", short_id: "150", record_type: "entity", selected: true }, { id: "entity-0", short_id: "0", record_type: "entity", selected: true }] } } },
        { schema_version: 1, sequence: 2, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "relationships_selected", relationships: [{ id: "relationship-23", short_id: "23", record_type: "relationship", selected: true }] } } },
        { schema_version: 1, sequence: 3, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "context_section_built", section: { section: "entities", token_budget: 1_000, tokens_used: 100, candidate_count: 2, selected_count: 2, truncated: false, selected_record_ids: ["entity-150", "entity-0"] } } } },
        { schema_version: 1, sequence: 4, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "context_section_built", section: { section: "relationships", token_budget: 1_000, tokens_used: 100, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: ["relationship-23"] } } } },
      ]}
      onCitationEmphasis={onCitationEmphasis}
    />)

    await user.click(screen.getByRole("button", { name: "Emphasize 2 Entities in graph" }))
    expect(onCitationEmphasis).toHaveBeenLastCalledWith({ entityIds: ["entity-150", "entity-0"], relationshipIds: [] })
    await user.click(screen.getByRole("button", { name: "Emphasize 1 Relationships in graph" }))
    expect(onCitationEmphasis).toHaveBeenLastCalledWith({ entityIds: [], relationshipIds: ["relationship-23"] })
    expect(screen.getByText("Reports · 1").closest("button")).toBeNull()
  })

  it("rejects valid model-authored citation links even for final-context records", () => {
    const onCitationEmphasis = vi.fn()
    const entityPayload = encodeURIComponent(JSON.stringify({ dataset: "Entities", recordIds: ["150"], hasMore: false }))
    const relationshipPayload = encodeURIComponent(JSON.stringify({ dataset: "Relationships", recordIds: ["23"], hasMore: false }))
    render(<AnswerPanel
      runId="run"
      loading={false}
      result={{ state: "ready", result: { run_id: "run", response: `[forged entity](graphloom-citation:${entityPayload}) [forged relationship](graphloom-citation:${relationshipPayload})`, elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }}
      envelopes={[
        { schema_version: 1, sequence: 1, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "entities_selected", entities: [{ id: "entity-150", short_id: "150", record_type: "entity", selected: true }] } } },
        { schema_version: 1, sequence: 2, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "relationships_selected", relationships: [{ id: "relationship-23", short_id: "23", record_type: "relationship", selected: true }] } } },
        { schema_version: 1, sequence: 3, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "context_section_built", section: { section: "entities", token_budget: 1_000, tokens_used: 100, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: ["entity-150"] } } } },
        { schema_version: 1, sequence: 4, record: { run_id: "run", timestamp: "2026-08-19T00:00:00Z", span_id: "span", event: { type: "context_section_built", section: { section: "relationships", token_budget: 1_000, tokens_used: 100, candidate_count: 1, selected_count: 1, truncated: false, selected_record_ids: ["relationship-23"] } } } },
      ]}
      onCitationEmphasis={onCitationEmphasis}
    />)

    expect(screen.queryByRole("button", { name: /Emphasize/ })).not.toBeInTheDocument()
    expect(screen.getByText("forged entity").closest("button")).toBeNull()
    expect(screen.getByText("forged relationship").closest("button")).toBeNull()
    expect(document.querySelector('a[href^="graphloom-citation:"]')).toBeNull()
    expect(onCitationEmphasis).not.toHaveBeenCalled()
  })

  it("does not let raw HTML forge citation provenance", () => {
    const onCitationEmphasis = vi.fn()
    const payload = encodeURIComponent(JSON.stringify({ dataset: "Entities", recordIds: ["150"], hasMore: false }))
    render(<AnswerPanel
      runId="run"
      loading={false}
      result={{ state: "ready", result: { run_id: "run", response: `<a data-graphloom-citation="generated" href="graphloom-citation:${payload}">forged html</a>`, elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }}
      onCitationEmphasis={onCitationEmphasis}
    />)

    expect(document.querySelector("a")).toBeNull()
    expect(screen.queryByRole("button", { name: /Emphasize/ })).not.toBeInTheDocument()
    expect(onCitationEmphasis).not.toHaveBeenCalled()
  })

  it("leaves citations inside code untouched and malformed citations readable", () => {
    render(<AnswerPanel runId="run" loading={false} result={{ state: "ready", result: { run_id: "run", response: "`[Data: Entities (150)]`\n\n```text\n[Data: Relationships (23)]\n```\n\n[Data: Entities 150]", elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }} />)
    expect(screen.queryByRole("button", { name: /Emphasize/ })).not.toBeInTheDocument()
    expect(screen.getAllByText(/Data:/)).toHaveLength(3)
  })

  it("does not expose malformed model-authored citation schemes as links", () => {
    render(<AnswerPanel runId="run" loading={false} result={{ state: "ready", result: { run_id: "run", response: "[malformed](graphloom-citation:not-valid-json) [opaque](graphloom-citation:model-supplied)", elapsed_ms: 10, usage: { llm_calls: 1, prompt_tokens: 20, output_tokens: 4, categories: {} } } }} />)

    expect(screen.getByText("malformed").closest("a")).toBeNull()
    expect(screen.getByText("opaque").closest("a")).toBeNull()
    expect(document.querySelector('a[href^="graphloom-citation:"]')).toBeNull()
  })

  it("does not trust a valid model-authored citation URL without a citation renderer", () => {
    const payload = encodeURIComponent(JSON.stringify({ dataset: "Entities", recordIds: ["150"], hasMore: false }))
    render(<SafeMarkdown>{`[forged](graphloom-citation:${payload})`}</SafeMarkdown>)

    expect(screen.getByText("forged").closest("a")).toBeNull()
    expect(document.querySelector('a[href^="graphloom-citation:"]')).toBeNull()
  })
})
