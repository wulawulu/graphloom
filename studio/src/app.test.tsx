import { render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { App } from "@/app"

afterEach(() => vi.unstubAllGlobals())

describe("App", () => {
  it("renders the Studio shell, Query composer, empty timeline, and unavailable graph state", async () => {
    vi.stubGlobal("fetch", vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (url.startsWith("/api/explainability/runs")) return Promise.resolve(new Response(JSON.stringify({ runs: [], next_cursor: null }), { status: 200 }))
      if (url.startsWith("/api/graph")) return Promise.resolve(new Response("unavailable", { status: 503 }))
      return Promise.resolve(new Response("missing", { status: 404 }))
    }))
    render(<App />)
    expect(screen.getByText("GraphLoom Studio")).toBeInTheDocument()
    expect(screen.getByLabelText("New Local Query")).toBeInTheDocument()
    expect(screen.getByText("No Run selected")).toBeInTheDocument()
    expect(await screen.findByText("Graph data unavailable")).toBeInTheDocument()
  })
})
