import type {
  CommunityListParameters,
  EntityListParameters,
  ExplainabilityRun,
  GraphCommunity,
  GraphCommunityReportDetail,
  GraphEntity,
  GraphEntityDetail,
  GraphListResponse,
  GraphOverviewParameters,
  GraphProjection,
  GraphRelationship,
  GraphRelationshipDetail,
  GraphSummary,
  GraphSubgraphRequest,
  QueryResultState,
  RelationshipListParameters,
  RunHistoryCursor,
  RunHistoryResponse,
  StartQueryRequest,
  StartQueryResponse,
  StudioQueryResult,
} from "@/api/types"

export class ApiError extends Error {
  readonly status: number

  constructor(status: number) {
    super("GraphLoom Studio request failed")
    this.name = "ApiError"
    this.status = status
  }
}

async function requestJson<T>(url: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers)
  headers.set("Accept", "application/json")
  const response = await fetch(url, {
    ...init,
    headers,
  })
  if (!response.ok) {
    throw new ApiError(response.status)
  }
  return response.json() as Promise<T>
}

function queryString(parameters: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(parameters)) {
    if (value !== undefined) {
      search.set(key, String(value))
    }
  }
  const encoded = search.toString()
  return encoded.length === 0 ? "" : `?${encoded}`
}

export function startQuery(body: StartQueryRequest, signal?: AbortSignal): Promise<StartQueryResponse> {
  return requestJson<StartQueryResponse>("/api/query", {
    method: "POST",
    body: JSON.stringify(body),
    headers: { "Content-Type": "application/json" },
    signal,
  })
}

export function getRun(runId: string, signal?: AbortSignal): Promise<ExplainabilityRun> {
  return requestJson<ExplainabilityRun>(`/api/explainability/runs/${encodeURIComponent(runId)}`, { signal })
}

export function listRuns(
  cursor?: RunHistoryCursor,
  signal?: AbortSignal,
  limit = 25,
): Promise<RunHistoryResponse> {
  const query = queryString({
    kind: "query",
    limit,
    before_started_at: cursor?.started_at,
    before_run_id: cursor?.run_id,
  })
  return requestJson<RunHistoryResponse>(`/api/explainability/runs${query}`, { signal })
}

export async function getQueryResult(runId: string, signal?: AbortSignal): Promise<QueryResultState> {
  const response = await fetch(`/api/query/${encodeURIComponent(runId)}/result`, {
    headers: { Accept: "application/json" },
    signal,
  })
  switch (response.status) {
    case 200:
      return { state: "ready", result: (await response.json()) as StudioQueryResult }
    case 202:
      return { state: "waiting" }
    case 409:
      return { state: "failed" }
    case 410:
      return { state: "gone" }
    case 404:
      return { state: "missing" }
    default:
      throw new ApiError(response.status)
  }
}

export function getGraphSummary(signal?: AbortSignal): Promise<GraphSummary> {
  return requestJson<GraphSummary>("/api/graph/summary", { signal })
}

export function getGraphOverview(
  parameters: GraphOverviewParameters = {},
  signal?: AbortSignal,
): Promise<GraphProjection> {
  return requestJson<GraphProjection>(`/api/graph/overview${queryString({
    max_entities: parameters.max_entities,
    max_relationships: parameters.max_relationships,
  })}`, { signal })
}

export function getGraphSubgraph(body: GraphSubgraphRequest, signal?: AbortSignal): Promise<GraphProjection> {
  return requestJson<GraphProjection>("/api/graph/subgraph", {
    method: "POST",
    body: JSON.stringify(body),
    headers: { "Content-Type": "application/json" },
    signal,
  })
}

export function listEntities(parameters: EntityListParameters, signal?: AbortSignal): Promise<GraphListResponse<GraphEntity>> {
  return requestJson<GraphListResponse<GraphEntity>>(`/api/graph/entities${queryString({
    type: parameters.type,
    community: parameters.community,
    limit: parameters.limit,
    after: parameters.after,
  })}`, { signal })
}

export function getEntity(id: string, signal?: AbortSignal): Promise<GraphEntityDetail> {
  return requestJson<GraphEntityDetail>(`/api/graph/entities/${encodeURIComponent(id)}`, { signal })
}

export function listRelationships(parameters: RelationshipListParameters, signal?: AbortSignal): Promise<GraphListResponse<GraphRelationship>> {
  return requestJson<GraphListResponse<GraphRelationship>>(`/api/graph/relationships${queryString({
    source: parameters.source,
    target: parameters.target,
    limit: parameters.limit,
    after: parameters.after,
  })}`, { signal })
}

export function getRelationship(id: string, signal?: AbortSignal): Promise<GraphRelationshipDetail> {
  return requestJson<GraphRelationshipDetail>(`/api/graph/relationships/${encodeURIComponent(id)}`, { signal })
}

export function listCommunities(parameters: CommunityListParameters, signal?: AbortSignal): Promise<GraphListResponse<GraphCommunity>> {
  return requestJson<GraphListResponse<GraphCommunity>>(`/api/graph/communities${queryString({
    level: parameters.level,
    parent: parameters.parent,
    limit: parameters.limit,
    after: parameters.after,
  })}`, { signal })
}

export function getCommunity(id: string, signal?: AbortSignal): Promise<GraphCommunity> {
  return requestJson<GraphCommunity>(`/api/graph/communities/${encodeURIComponent(id)}`, { signal })
}

export function getCommunityReport(id: string, signal?: AbortSignal): Promise<GraphCommunityReportDetail> {
  return requestJson<GraphCommunityReportDetail>(`/api/graph/communities/${encodeURIComponent(id)}/report`, { signal })
}
