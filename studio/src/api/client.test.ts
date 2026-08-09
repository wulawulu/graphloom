import { afterEach, describe, expect, it, vi } from "vitest"

import { getGraphOverview, getGraphSubgraph, getGraphSummary, getQueryResult, listRuns, startQuery } from "@/api/client"

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
      .mockResolvedValueOnce(response(202, { run_id: "run-1", run_url: "/run", events_url: "/events", result_url: "/result" }))
      .mockResolvedValueOnce(response(200, { run_id: "run-1", response: "answer", elapsed_ms: 3, usage: { llm_calls: 1, prompt_tokens: 2, output_tokens: 3, categories: {} } }))
      .mockResolvedValueOnce(response(202, "waiting"))
      .mockResolvedValueOnce(response(409, "failed"))
      .mockResolvedValueOnce(response(410, "gone"))
    vi.stubGlobal("fetch", fetchMock)

    const accepted = await startQuery({ query: "fixture", method: "local", content_mode: "metadata", response_type: "Multiple Paragraphs" })
    expect(accepted.run_id).toBe("run-1")
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
})
