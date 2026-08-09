import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { AnswerPanel } from "@/components/result/answer-panel"

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
})
