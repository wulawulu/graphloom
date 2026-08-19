import type { ExplainabilityCandidate, ExplainabilityEnvelope } from "@/api/types"

export interface CitationGroup {
  dataset: string
  recordIds: string[]
  hasMore: boolean
}

export interface DataCitation {
  raw: string
  groups: CitationGroup[]
}

export interface CitationGraphIndex {
  entities: Map<string, string>
  relationships: Map<string, string>
}

export interface GraphEmphasis {
  entityIds: string[]
  relationshipIds: string[]
}

export type GraphEmphasisIntent = GraphEmphasis & { revision: number }

export const CITATION_URL_PREFIX = "graphloom-citation:"

const DATA_CITATION_PATTERN = /\[Data:\s*[^\]\r\n]*\]/g
const GROUP_PATTERN = /([^()[\],;\r\n]+?)\s*\(([^()]*)\)/g

export function parseDataCitation(raw: string): DataCitation | null {
  if (!raw.startsWith("[Data:") || !raw.endsWith("]")) return null
  const body = raw.slice(6, -1)
  const groups: CitationGroup[] = []
  let cursor = 0

  for (const match of body.matchAll(GROUP_PATTERN)) {
    const index = match.index
    if (index === undefined || !isGroupSeparator(body.slice(cursor, index))) return null
    const dataset = match[1]?.trim()
    const records = match[2]?.split(",").map((value) => value.trim()) ?? []
    if (dataset === undefined || dataset.length === 0 || records.length === 0 || records.some((value) => value.length === 0)) return null
    const hasMore = records.includes("+more")
    const recordIds = records.filter((value) => value !== "+more")
    if (recordIds.length === 0) return null
    groups.push({ dataset, recordIds, hasMore })
    cursor = index + match[0].length
  }

  if (groups.length === 0 || !isGroupSeparator(body.slice(cursor))) return null
  return { raw, groups }
}

function isGroupSeparator(value: string): boolean {
  return /^[\s,;]*$/.test(value)
}

export function buildCitationGraphIndex(envelopes: readonly ExplainabilityEnvelope[]): CitationGraphIndex {
  const entityIdentities = new Map<string, string | null>()
  const relationshipIdentities = new Map<string, string | null>()
  const finalEntityIds = new Set<string>()
  const finalRelationshipIds = new Set<string>()

  for (const envelope of envelopes) {
    const event = envelope.record.event
    if (event.type === "entities_selected") addCandidates(entityIdentities, event.entities)
    if (event.type === "relationships_selected") addCandidates(relationshipIdentities, event.relationships)
    if (event.type === "context_section_built") addContextMembership(event.section, finalEntityIds, finalRelationshipIds)
  }

  return {
    entities: uniqueMappings(entityIdentities, finalEntityIds),
    relationships: uniqueMappings(relationshipIdentities, finalRelationshipIds),
  }
}

function addContextMembership(value: unknown, entityIds: Set<string>, relationshipIds: Set<string>): void {
  if (typeof value !== "object" || value === null) return
  const section = value as { section?: unknown; selected_record_ids?: unknown }
  if (!Array.isArray(section.selected_record_ids) || !section.selected_record_ids.every((id) => typeof id === "string")) return
  const target = section.section === "entities"
    ? entityIds
    : section.section === "relationships"
      ? relationshipIds
      : null
  if (target === null) return
  section.selected_record_ids.forEach((id) => target.add(id))
}

function addCandidates(target: Map<string, string | null>, value: unknown): void {
  if (!Array.isArray(value)) return
  for (const item of value) {
    if (!isSelectedCandidate(item) || item.short_id === undefined) continue
    const current = target.get(item.short_id)
    if (current === undefined) target.set(item.short_id, item.id)
    else if (current !== item.id) target.set(item.short_id, null)
  }
}

function isSelectedCandidate(value: unknown): value is ExplainabilityCandidate & { short_id?: string } {
  if (typeof value !== "object" || value === null) return false
  const candidate = value as Partial<ExplainabilityCandidate>
  return candidate.selected === true
    && typeof candidate.id === "string"
    && (candidate.short_id === undefined || typeof candidate.short_id === "string")
}

function uniqueMappings(values: Map<string, string | null>, finalContextIds: ReadonlySet<string>): Map<string, string> {
  return new Map([...values].flatMap(([shortId, stableId]) => stableId === null || !finalContextIds.has(stableId) ? [] : [[shortId, stableId]]))
}

export function resolveCitationGroup(group: CitationGroup, index: CitationGraphIndex): GraphEmphasis | null {
  const dataset = group.dataset.toLowerCase()
  const entityIds = dataset === "entities" ? resolveRecordIds(group.recordIds, index.entities) : []
  const relationshipIds = dataset === "relationships" ? resolveRecordIds(group.recordIds, index.relationships) : []
  return entityIds.length === 0 && relationshipIds.length === 0 ? null : { entityIds, relationshipIds }
}

function resolveRecordIds(recordIds: readonly string[], mappings: ReadonlyMap<string, string>): string[] {
  const resolved: string[] = []
  const seen = new Set<string>()
  for (const recordId of recordIds) {
    const stableId = mappings.get(recordId)
    if (stableId === undefined || seen.has(stableId)) continue
    seen.add(stableId)
    resolved.push(stableId)
  }
  return resolved
}

interface MarkdownNode {
  type: string
  value?: string
  url?: string
  children?: MarkdownNode[]
}

export function remarkDataCitations(): (tree: MarkdownNode) => void {
  return (tree) => transformChildren(tree)
}

function transformChildren(parent: MarkdownNode): void {
  if (parent.children === undefined) return
  const transformed: MarkdownNode[] = []
  for (const child of parent.children) {
    if (child.type !== "text" || child.value === undefined) {
      transformChildren(child)
      transformed.push(child)
      continue
    }
    transformed.push(...citationTextNodes(child.value))
  }
  parent.children = transformed
}

function citationTextNodes(value: string): MarkdownNode[] {
  const nodes: MarkdownNode[] = []
  let cursor = 0
  for (const match of value.matchAll(DATA_CITATION_PATTERN)) {
    const index = match.index
    const raw = match[0]
    if (index === undefined) continue
    if (index > cursor) nodes.push({ type: "text", value: value.slice(cursor, index) })
    const citation = parseDataCitation(raw)
    if (citation === null) {
      nodes.push({ type: "text", value: raw })
    } else {
      citation.groups.forEach((group, groupIndex) => {
        if (groupIndex > 0) nodes.push({ type: "text", value: " " })
        nodes.push({
          type: "link",
          url: `${CITATION_URL_PREFIX}${encodeURIComponent(JSON.stringify(group))}`,
          children: [{ type: "text", value: `${group.dataset} · ${group.recordIds.length}${group.hasMore ? "+" : ""}` }],
        })
      })
    }
    cursor = index + raw.length
  }
  if (cursor < value.length) nodes.push({ type: "text", value: value.slice(cursor) })
  return nodes.length === 0 ? [{ type: "text", value }] : nodes
}

export function citationGroupFromUrl(url: string): CitationGroup | null {
  if (!url.startsWith(CITATION_URL_PREFIX)) return null
  try {
    const value: unknown = JSON.parse(decodeURIComponent(url.slice(CITATION_URL_PREFIX.length)))
    if (typeof value !== "object" || value === null) return null
    const group = value as Partial<CitationGroup>
    if (
      typeof group.dataset !== "string"
      || group.dataset.length === 0
      || group.dataset.trim() !== group.dataset
      || !Array.isArray(group.recordIds)
      || group.recordIds.length === 0
      || !group.recordIds.every((id) => typeof id === "string" && id.length > 0 && id.trim() === id)
      || typeof group.hasMore !== "boolean"
    ) return null
    return { dataset: group.dataset, recordIds: group.recordIds, hasMore: group.hasMore }
  } catch {
    return null
  }
}
