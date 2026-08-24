//! Shared deterministic resolution of graph references inside one snapshot.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    GraphCommunity, GraphCommunityRef, GraphCommunityReportDetail, GraphDataSnapshot,
    GraphEntityDetail, GraphEntityRef, GraphRelationshipDetail, GraphTextUnitDetail,
    GraphTextUnitRef,
};

const TEXT_UNIT_PREVIEW_CHAR_LIMIT: usize = 240;

/// Snapshot-local resolver shared by projections and Inspector detail enrichment.
#[derive(Debug)]
pub(crate) struct GraphReferenceIndex<'a> {
    entities_by_title: BTreeMap<&'a str, Vec<&'a GraphEntityDetail>>,
    communities_by_short_id: BTreeMap<&'a str, &'a GraphCommunity>,
    communities_by_number: BTreeMap<i64, Vec<&'a GraphCommunity>>,
    reports_by_community_id: BTreeMap<&'a str, &'a GraphCommunityReportDetail>,
}

impl<'a> GraphReferenceIndex<'a> {
    pub(crate) fn new(snapshot: &'a GraphDataSnapshot) -> Self {
        let mut entities_by_title = BTreeMap::<&str, Vec<&GraphEntityDetail>>::new();
        for entity in &snapshot.entities {
            entities_by_title
                .entry(entity.title.as_str())
                .or_default()
                .push(entity);
        }
        Self {
            entities_by_title,
            communities_by_short_id: snapshot
                .communities
                .iter()
                .map(|community| (community.short_id.as_str(), community))
                .collect(),
            communities_by_number: {
                let mut communities = BTreeMap::<i64, Vec<&GraphCommunity>>::new();
                for community in &snapshot.communities {
                    if let Ok(id) = community.short_id.parse() {
                        communities.entry(id).or_default().push(community);
                    }
                }
                communities
            },
            reports_by_community_id: snapshot
                .community_reports
                .iter()
                .map(|report| (report.community_id.as_str(), report))
                .collect(),
        }
    }

    /// Resolve both relationship endpoint titles only when each title is unique.
    pub(crate) fn relationship_endpoints(
        &self,
        relationship: &GraphRelationshipDetail,
    ) -> Option<(&'a GraphEntityDetail, &'a GraphEntityDetail)> {
        Some((
            self.unique_entity(relationship.source.as_str())?,
            self.unique_entity(relationship.target.as_str())?,
        ))
    }

    pub(crate) fn enrich_entity(&self, entity: &GraphEntityDetail) -> GraphEntityDetail {
        let mut value = entity.clone();
        value.communities = entity
            .community_ids
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| self.communities_by_short_id.get(id.as_str()).copied())
            .map(|community| self.community_reference(community))
            .collect();
        value
    }

    pub(crate) fn enrich_relationship(
        &self,
        relationship: &GraphRelationshipDetail,
    ) -> GraphRelationshipDetail {
        let mut value = relationship.clone();
        value.source_entity = self
            .unique_entity(relationship.source.as_str())
            .map(GraphEntityRef::from);
        value.target_entity = self
            .unique_entity(relationship.target.as_str())
            .map(GraphEntityRef::from);
        value
    }

    pub(crate) fn enrich_community(&self, community: &GraphCommunity) -> GraphCommunity {
        let mut value = GraphCommunity::with_report(
            community,
            self.reports_by_community_id
                .get(community.short_id.as_str())
                .copied(),
        );
        value.parent_community = (community.parent >= 0)
            .then_some(community.parent)
            .and_then(|id| self.unique_community(id))
            .map(|parent| self.community_reference(parent));
        value.child_communities = community
            .children
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| self.unique_community(id))
            .map(|child| self.community_reference(child))
            .collect();
        value
    }

    fn community_reference(&self, community: &GraphCommunity) -> GraphCommunityRef {
        let report = self
            .reports_by_community_id
            .get(community.short_id.as_str());
        GraphCommunityRef {
            id: community.id.clone(),
            short_id: community.short_id.clone(),
            title: community.title.clone(),
            report_title: report.map(|report| report.title.clone()),
            level: community.level,
            summary: report.map(|report| report.summary.clone()),
        }
    }

    fn unique_entity(&self, title: &str) -> Option<&'a GraphEntityDetail> {
        let [entity] = self.entities_by_title.get(title)?.as_slice() else {
            return None;
        };
        Some(*entity)
    }

    fn unique_community(&self, id: i64) -> Option<&'a GraphCommunity> {
        let [community] = self.communities_by_number.get(&id)?.as_slice() else {
            return None;
        };
        Some(*community)
    }
}

impl From<&GraphEntityDetail> for GraphEntityRef {
    fn from(entity: &GraphEntityDetail) -> Self {
        Self {
            id: entity.id.clone(),
            short_id: entity.short_id.clone(),
            title: entity.title.clone(),
            entity_type: entity.entity_type.clone(),
            description: entity.description.clone(),
        }
    }
}

/// Stable-id resolver for a single batch of text-unit evidence.
#[derive(Debug)]
pub(crate) struct GraphTextUnitIndex<'a> {
    by_id: BTreeMap<&'a str, Vec<&'a GraphTextUnitDetail>>,
}

impl<'a> GraphTextUnitIndex<'a> {
    pub(crate) fn new(text_units: &'a [GraphTextUnitDetail]) -> Self {
        let mut by_id = BTreeMap::<&str, Vec<&GraphTextUnitDetail>>::new();
        for text_unit in text_units {
            by_id
                .entry(text_unit.id.as_str())
                .or_default()
                .push(text_unit);
        }
        Self { by_id }
    }

    pub(crate) fn references(&self, ids: &[String]) -> Vec<GraphTextUnitRef> {
        let mut seen = BTreeSet::new();
        ids.iter()
            .filter(|id| seen.insert(id.as_str()))
            .filter_map(|id| self.unique(id))
            .map(|text_unit| GraphTextUnitRef {
                id: text_unit.id.clone(),
                short_id: text_unit.short_id.clone(),
                preview: text_unit_preview(&text_unit.text),
                n_tokens: text_unit.n_tokens,
            })
            .collect()
    }

    pub(crate) fn detail(&self, id: &str) -> Option<GraphTextUnitDetail> {
        self.unique(id).cloned()
    }

    fn unique(&self, id: &str) -> Option<&'a GraphTextUnitDetail> {
        let [text_unit] = self.by_id.get(id)?.as_slice() else {
            return None;
        };
        Some(*text_unit)
    }
}

fn text_unit_preview(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let mut preview = chars
        .by_ref()
        .take(TEXT_UNIT_PREVIEW_CHAR_LIMIT)
        .collect::<String>();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::graph::{GraphDataSourceError, subgraph};

    type TestResult = Result<(), Box<dyn Error>>;

    fn entity(id: &str, title: &str) -> GraphEntityDetail {
        GraphEntityDetail::new(id.to_owned(), title.to_owned())
            .with_short_id(format!("short-{id}"))
            .with_entity_type("PERSON".to_owned())
            .with_description(format!("Description for {title}"))
    }

    fn relationship(id: &str, source: &str, target: &str) -> GraphRelationshipDetail {
        GraphRelationshipDetail::new(id.to_owned(), source.to_owned(), target.to_owned())
    }

    fn community(id: &str, short_id: &str, level: i64) -> GraphCommunity {
        GraphCommunity::new(
            id.to_owned(),
            short_id.to_owned(),
            format!("Community {short_id}"),
        )
        .with_hierarchy(level, -1, Vec::new())
    }

    fn text_unit(id: &str, short_id: &str, text: &str) -> GraphTextUnitDetail {
        GraphTextUnitDetail::new(id.to_owned(), short_id.to_owned(), text.to_owned())
            .with_n_tokens(42)
            .with_document_id(format!("document-{id}"))
    }

    fn snapshot(
        entities: Vec<GraphEntityDetail>,
        relationships: Vec<GraphRelationshipDetail>,
        communities: Vec<GraphCommunity>,
        reports: Vec<GraphCommunityReportDetail>,
    ) -> Result<GraphDataSnapshot, GraphDataSourceError> {
        GraphDataSnapshot::new(entities, relationships, communities, reports)
    }

    #[test]
    fn test_should_resolve_unique_relationship_endpoints_and_match_projection() -> TestResult {
        let data = snapshot(
            vec![entity("a", "Alice"), entity("b", "Bob")],
            vec![
                relationship("r", "Alice", "Bob"),
                relationship("self", "Alice", "Alice"),
            ],
            Vec::new(),
            Vec::new(),
        )?;
        let index = GraphReferenceIndex::new(&data);
        let resolved = index.enrich_relationship(&data.relationships[0]);
        assert_eq!(
            resolved
                .source_entity
                .as_ref()
                .map(|value| value.id.as_str()),
            Some("a")
        );
        assert_eq!(
            resolved
                .target_entity
                .as_ref()
                .map(|value| value.id.as_str()),
            Some("b")
        );

        let self_relationship = index.enrich_relationship(&data.relationships[1]);
        assert_eq!(
            self_relationship
                .source_entity
                .as_ref()
                .map(|value| value.id.as_str()),
            Some("a")
        );
        assert_eq!(
            self_relationship
                .target_entity
                .as_ref()
                .map(|value| value.id.as_str()),
            Some("a")
        );

        let projection = subgraph(&data, &[], &["r".to_owned()], 0, 10, 10)?;
        let edge = projection
            .relationships
            .first()
            .ok_or("projected relationship")?;
        assert_eq!(
            edge.source_entity_id,
            resolved.source_entity.ok_or("source ref")?.id
        );
        assert_eq!(
            edge.target_entity_id,
            resolved.target_entity.ok_or("target ref")?.id
        );
        Ok(())
    }

    #[test]
    fn test_should_leave_ambiguous_or_missing_relationship_endpoints_unresolved() -> TestResult {
        for (entities, source, target, expected_source, expected_target) in [
            (
                vec![
                    entity("a1", "Alice"),
                    entity("a2", "Alice"),
                    entity("b", "Bob"),
                ],
                "Alice",
                "Bob",
                None,
                Some("b"),
            ),
            (
                vec![
                    entity("a", "Alice"),
                    entity("b1", "Bob"),
                    entity("b2", "Bob"),
                ],
                "Alice",
                "Bob",
                Some("a"),
                None,
            ),
            (
                vec![entity("a", "Alice")],
                "Alice",
                "Missing",
                Some("a"),
                None,
            ),
        ] {
            let data = snapshot(
                entities,
                vec![relationship("r", source, target)],
                Vec::new(),
                Vec::new(),
            )?;
            let resolved =
                GraphReferenceIndex::new(&data).enrich_relationship(&data.relationships[0]);
            assert_eq!(
                resolved
                    .source_entity
                    .as_ref()
                    .map(|value| value.id.as_str()),
                expected_source
            );
            assert_eq!(
                resolved
                    .target_entity
                    .as_ref()
                    .map(|value| value.id.as_str()),
                expected_target
            );
            let projection = subgraph(&data, &[], &["r".to_owned()], 0, 10, 10)?;
            assert!(projection.relationships.is_empty());
            assert_eq!(projection.unresolved_relationship_ids, ["r"]);
        }
        Ok(())
    }

    #[test]
    fn test_should_resolve_entity_communities_deterministically_and_preserve_raw_memberships()
    -> TestResult {
        let member = entity("a", "Alice").with_community_ids(vec![
            "9".to_owned(),
            "2".to_owned(),
            "missing".to_owned(),
            "2".to_owned(),
        ]);
        let data = snapshot(
            vec![member],
            Vec::new(),
            vec![
                community("community-9", "9", 1),
                community("community-2", "2", 0),
            ],
            vec![GraphCommunityReportDetail::new(
                "report-2".to_owned(),
                "2".to_owned(),
                "Report two".to_owned(),
                "Summary two".to_owned(),
                "Full two".to_owned(),
            )],
        )?;
        let detail = GraphReferenceIndex::new(&data).enrich_entity(&data.entities[0]);
        assert_eq!(detail.community_ids, ["9", "2", "missing", "2"]);
        assert_eq!(
            detail
                .communities
                .iter()
                .map(|value| value.id.as_str())
                .collect::<Vec<_>>(),
            ["community-2", "community-9"]
        );
        assert_eq!(
            detail.communities[0].summary.as_deref(),
            Some("Summary two")
        );
        assert_eq!(
            detail.communities[0].report_title.as_deref(),
            Some("Report two")
        );
        assert!(detail.communities[1].report_title.is_none());
        assert!(detail.communities[1].summary.is_none());
        Ok(())
    }

    #[test]
    fn test_should_resolve_community_hierarchy_without_failing_on_missing_references() -> TestResult
    {
        let root = community("root", "1", 1).with_hierarchy(1, -1, vec![3, 2, 99, 2]);
        let child_two = community("child-2", "2", 0).with_hierarchy(0, 1, Vec::new());
        let child_three = community("child-3", "3", 0).with_hierarchy(0, 1, Vec::new());
        let orphan = community("orphan", "4", 0).with_hierarchy(0, 88, Vec::new());
        let data = snapshot(
            Vec::new(),
            Vec::new(),
            vec![root, child_two, child_three, orphan],
            Vec::new(),
        )?;
        let index = GraphReferenceIndex::new(&data);
        let root_detail = index.enrich_community(&data.communities[0]);
        assert!(root_detail.parent_community.is_none());
        assert_eq!(root_detail.children, [3, 2, 99, 2]);
        assert_eq!(
            root_detail
                .child_communities
                .iter()
                .map(|value| value.id.as_str())
                .collect::<Vec<_>>(),
            ["child-2", "child-3"]
        );
        let child_detail = index.enrich_community(&data.communities[1]);
        assert_eq!(
            child_detail
                .parent_community
                .as_ref()
                .map(|value| value.id.as_str()),
            Some("root")
        );
        assert!(child_detail.child_communities.is_empty());
        let orphan_detail = index.enrich_community(&data.communities[3]);
        assert!(orphan_detail.parent_community.is_none());
        assert_eq!(orphan_detail.parent, 88);
        Ok(())
    }

    #[test]
    fn test_should_resolve_text_units_in_first_seen_artifact_order() {
        let units = vec![
            text_unit("text-1", "1", "First source"),
            text_unit("text-2", "2", "Second source"),
        ];
        let index = GraphTextUnitIndex::new(&units);
        let ids = ["text-2", "missing", "text-1", "text-2"].map(str::to_owned);

        let references = index.references(&ids);

        assert_eq!(
            references
                .iter()
                .map(|reference| reference.id.as_str())
                .collect::<Vec<_>>(),
            ["text-2", "text-1"]
        );
        assert_eq!(index.detail("text-1"), Some(units[0].clone()));
        assert!(index.detail("missing").is_none());
    }

    #[test]
    fn test_should_build_unicode_safe_deterministic_previews_without_changing_detail_text() {
        let exact = format!("  西门庆\n\n通过杨提督   行贿。{}", "证".repeat(260));
        let unit = text_unit("text-1", "1", &exact);
        let references =
            GraphTextUnitIndex::new(std::slice::from_ref(&unit)).references(&["text-1".to_owned()]);
        let preview = &references[0].preview;

        assert!(preview.starts_with("西门庆 通过杨提督 行贿。"));
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), TEXT_UNIT_PREVIEW_CHAR_LIMIT + 1);
        assert!(!preview.contains(char::REPLACEMENT_CHARACTER));
        assert_eq!(unit.text, exact);
        assert_eq!(format!("{unit:?}"), "GraphTextUnitDetail { .. }");
        assert!(!format!("{:?}", references[0]).contains("西门庆"));
    }
}
