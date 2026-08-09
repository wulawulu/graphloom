import type { ElementDefinition } from "cytoscape"

import type { GraphEntity, GraphRelationship } from "@/api/types"

export interface HighlightableGraph {
  batch(callback: () => void): void
  elements(): { removeClass(name: string): void }
  getElementById(id: string): { addClass(name: string): void }
}

export function applyGraphHighlights(
  graph: HighlightableGraph,
  entityIds: ReadonlySet<string>,
  relationshipIds: ReadonlySet<string>,
): void {
  graph.batch(() => {
    graph.elements().removeClass("highlighted")
    for (const id of [...entityIds, ...relationshipIds]) {
      graph.getElementById(id).addClass("highlighted")
    }
  })
}

export interface GraphPreviewElements {
  elements: ElementDefinition[]
  resolvedRelationshipCount: number
  omittedRelationshipCount: number
}

export function buildGraphElements(
  entities: readonly GraphEntity[],
  relationships: readonly GraphRelationship[],
): GraphPreviewElements {
  const byTitle = new Map<string, GraphEntity[]>()
  for (const entity of entities) {
    const existing = byTitle.get(entity.title) ?? []
    existing.push(entity)
    byTitle.set(entity.title, existing)
  }

  const elements: ElementDefinition[] = entities.map((entity) => ({
    group: "nodes",
    data: { id: entity.id, label: entity.title, entityType: entity.entity_type ?? "untyped" },
  }))
  let resolvedRelationshipCount = 0
  for (const relationship of relationships) {
    const sources = byTitle.get(relationship.source)
    const targets = byTitle.get(relationship.target)
    if (sources?.length !== 1 || targets?.length !== 1) continue
    const source = sources[0]
    const target = targets[0]
    if (source === undefined || target === undefined) continue
    elements.push({
      group: "edges",
      data: {
        id: relationship.id,
        source: source.id,
        target: target.id,
        label: relationship.short_id ?? "",
      },
    })
    resolvedRelationshipCount += 1
  }

  return {
    elements,
    resolvedRelationshipCount,
    omittedRelationshipCount: relationships.length - resolvedRelationshipCount,
  }
}
