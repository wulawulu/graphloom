export type RunStatus = string
export type QueryMethod = string
export type ContentMode = "metadata" | "content" | "debug"

export interface StartQueryRequest {
  query: string
  method: "local"
  content_mode: ContentMode
  response_type: string
}

export interface StartQueryResponse {
  run_id: string
  run_url: string
  events_url: string
  result_url: string
}

export interface ExplainabilityRun {
  run_id: string
  kind: string
  status: RunStatus
  query?: string
  query_method?: QueryMethod
  started_at: string
  completed_at?: string
  compatibility_profile?: string
  event_count: number
  [key: string]: unknown
}

export interface RunHistoryCursor {
  started_at: string
  run_id: string
}

export interface RunHistoryResponse {
  runs: ExplainabilityRun[]
  next_cursor: RunHistoryCursor | null
}

export interface StudioQueryUsageCategory {
  llm_calls: number
  prompt_tokens: number
  output_tokens: number
}

export interface StudioQueryUsage extends StudioQueryUsageCategory {
  categories: Record<string, StudioQueryUsageCategory>
}

export interface StudioQueryResult {
  run_id: string
  response: string
  elapsed_ms: number
  usage: StudioQueryUsage
}

export type QueryResultState =
  | { state: "ready"; result: StudioQueryResult }
  | { state: "waiting" }
  | { state: "failed" }
  | { state: "gone" }
  | { state: "missing" }

export interface ExplainabilityEventPayload {
  type: string
  [key: string]: unknown
}

export interface ExplainabilityCandidate {
  id: string
  short_id?: string
  title?: string
  record_type: string
  score?: number
  rank?: number
  selected: boolean
  reason?: string
  source_id?: string
  relationship_id?: string
  expansion_depth?: number
}

export interface ExplainabilityContextSection {
  section: string
  name?: string
  token_budget: number
  tokens_used: number
  candidate_count: number
  selected_count: number
  truncated: boolean
  selected_record_ids: string[]
}

export interface ExplainabilityRecord {
  run_id: string
  timestamp: string
  span_id: string
  parent_span_id?: string
  event: ExplainabilityEventPayload
}

export interface ExplainabilityEnvelope {
  schema_version: number
  sequence: number
  record: ExplainabilityRecord
}

export interface GraphSummary {
  entity_count: number
  relationship_count: number
  community_count: number
  community_report_count: number
  community_levels: number[]
  entity_types: Record<string, number>
  untyped_entity_count: number
}

export interface GraphProjectionEntity {
  id: string
  title: string
  entity_type: string | null
  rank: number | null
}

export interface GraphProjectionRelationship {
  id: string
  source_entity_id: string
  target_entity_id: string
  source: string
  target: string
  weight: number | null
  rank: number | null
}

export interface GraphProjection {
  entities: GraphProjectionEntity[]
  relationships: GraphProjectionRelationship[]
  seed_entity_ids: string[]
  seed_relationship_ids: string[]
  missing_entity_ids: string[]
  missing_relationship_ids: string[]
  unresolved_relationship_ids: string[]
  unresolved_relationship_count: number
  truncated: boolean
}

export interface GraphOverviewParameters {
  max_entities?: number
  max_relationships?: number
}

export interface GraphSubgraphRequest {
  entity_ids: string[]
  relationship_ids: string[]
  depth?: 0 | 1
  max_entities?: number
  max_relationships?: number
}

export interface GraphEntity {
  id: string
  short_id: string | null
  title: string
  entity_type: string | null
  rank: number | null
  community_ids: string[]
}

export interface GraphEntityDetail extends GraphEntity {
  description: string | null
  text_unit_ids: string[]
}

export interface GraphRelationship {
  id: string
  short_id: string | null
  source: string
  target: string
  weight: number | null
  rank: number | null
}

export interface GraphRelationshipDetail extends GraphRelationship {
  description: string | null
  text_unit_ids: string[]
}

export interface GraphCommunityReportSummary {
  id: string
  short_id: string
  community_id: string
  title: string
  summary: string
  rank: number | null
}

export interface GraphCommunityReportDetail extends GraphCommunityReportSummary {
  full_content: string
}

export interface GraphCommunity {
  id: string
  short_id: string
  title: string
  level: number
  parent: number
  children: number[]
  report: GraphCommunityReportSummary | null
}

export interface GraphListResponse<T> {
  items: T[]
  next_cursor: string | null
}

export interface EntityListParameters {
  type?: string
  community?: string
  limit?: number
  after?: string
}

export interface RelationshipListParameters {
  source?: string
  target?: string
  limit?: number
  after?: string
}

export interface CommunityListParameters {
  level?: number
  parent?: number
  limit?: number
  after?: string
}
