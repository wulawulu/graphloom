import { afterEach, describe, expect, it, vi } from "vitest"

import { getGraphOverview, getGraphSubgraph, getGraphSummary, getQueryResult, getTextUnit, listRuns, resolveTextUnits, startQuery } from "@/api/client"

function response(status: number, body: unknown): Response {
  return new Response(typeof body === "string" ? body : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

afterEach(() => vi.unstubAllGlobals())

describe("Studio API client", () => {
  it("handles accepted Query and all result lifecycle statuses", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(response(202, { run_id: "run-1", run_url: "/run", explainability_events_url: "/explainability-events", answer_events_url: "/answer-events", result_url: "/result" }))
      .mockResolvedValueOnce(response(200, { run_id: "run-1", response: "answer", elapsed_ms: 3, usage: { llm_calls: 1, prompt_tokens: 2, output_tokens: 3, categories: {} } }))
      .mockResolvedValueOnce(response(202, "waiting"))
      .mockResolvedValueOnce(response(409, "failed"))
      .mockResolvedValueOnce(response(410, "gone"))
    vi.stubGlobal("fetch", fetchMock)

    const accepted = await startQuery({ query: "fixture", method: "local", dynamic_community_selection: false, content_mode: "metadata", response_type: "Multiple Paragraphs" })
    expect(accepted.run_id).toBe("run-1")
    expect(JSON.parse(String((fetchMock.mock.calls[0]?.[1] as RequestInit).body))).toEqual({
      query: "fixture",
      method: "local",
      dynamic_community_selection: false,
      content_mode: "metadata",
      response_type: "Multiple Paragraphs",
    })
    expect((await getQueryResult("run-1")).state).toBe("ready")
    expect((await getQueryResult("run-1")).state).toBe("waiting")
    expect((await getQueryResult("run-1")).state).toBe("failed")
    expect((await getQueryResult("run-1")).state).toBe("gone")
  })

  it("passes the backend run cursor without inventing a new cursor", async () => {
    const fetchMock = vi.fn().mockResolvedValue(response(200, { runs: [], next_cursor: null }))
    vi.stubGlobal("fetch", fetchMock)
    await listRuns({ started_at: "2026-08-09T01:02:03.123456789Z", run_id: "run-9" })
    const url = String(fetchMock.mock.calls[0]?.[0])
    expect(url).toContain("before_started_at=2026-08-09T01%3A02%3A03.123456789Z")
    expect(url).toContain("before_run_id=run-9")
  })

  it("keeps graph service failures low-information", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response(503, "GRAPH_PATH_SECRET_SENTINEL")))
    await expect(getGraphSummary()).rejects.toMatchObject({ status: 503 })
    await expect(getGraphSummary()).rejects.not.toThrow("GRAPH_PATH_SECRET_SENTINEL")
  })

  it("uses the bounded graph projection contracts", async () => {
    const projection = { entities: [], relationships: [], seed_entity_ids: [], seed_relationship_ids: [], missing_entity_ids: [], missing_relationship_ids: [], unresolved_relationship_ids: [], unresolved_relationship_count: 0, truncated: false }
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(response(200, projection)))
    vi.stubGlobal("fetch", fetchMock)
    await getGraphOverview({ max_entities: 20, max_relationships: 30 })
    await getGraphSubgraph({ entity_ids: ["entity-1"], relationship_ids: ["relationship-1"], depth: 1, max_entities: 80, max_relationships: 160 })

    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/graph/overview?max_entities=20&max_relationships=30")
    expect(fetchMock.mock.calls[1]?.[0]).toBe("/api/graph/subgraph")
    const init = fetchMock.mock.calls[1]?.[1] as RequestInit
    expect(init.method).toBe("POST")
    expect(JSON.parse(String(init.body))).toEqual({ entity_ids: ["entity-1"], relationship_ids: ["relationship-1"], depth: 1, max_entities: 80, max_relationships: 160 })
  })

  it("loads exact text-unit evidence by stable ID", async () => {
    const fetchMock = vi.fn().mockResolvedValue(response(200, { id: "text/unit", short_id: "184", text: "Exact", n_tokens: 3, document_id: null }))
    vi.stubGlobal("fetch", fetchMock)

    await expect(getTextUnit("text/unit")).resolves.toMatchObject({ text: "Exact" })
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/graph/text-units/text%2Funit")
  })

  it("resolves lightweight text-unit evidence as one stable-ID batch", async () => {
    const payload = { resolved: [{ id: "text-2", short_id: "206", preview: "Evidence", n_tokens: 8 }], missing_ids: ["missing"] }
    const fetchMock = vi.fn().mockResolvedValue(response(200, payload))
    vi.stubGlobal("fetch", fetchMock)

    await expect(resolveTextUnits(["text-2", "missing"])).resolves.toEqual(payload)
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/graph/text-units/resolve")
    const init = fetchMock.mock.calls[0]?.[1] as RequestInit
    expect(init.method).toBe("POST")
    expect(JSON.parse(String(init.body))).toEqual({ ids: ["text-2", "missing"] })
  })
})
