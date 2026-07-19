//! Immutable lookup indexes over adapted Query data.

use std::collections::HashMap;

use super::{CommunityReport, Covariate, Entity, Relationship, TextUnit};

/// Lookup-only indexes whose values retain original table positions.
///
/// Hash-map iteration order is never used to render context. Callers resolve positions back into
/// the original vectors and retain the compatibility ordering of those vectors.
#[derive(Debug)]
pub(crate) struct QueryDataIndex {
    pub(crate) entity_by_id: HashMap<String, usize>,
    pub(crate) entity_by_title: HashMap<String, Vec<usize>>,
    pub(crate) report_by_community_id: HashMap<String, usize>,
    pub(crate) text_unit_by_id: HashMap<String, usize>,
    pub(crate) relationships_by_entity: HashMap<String, Vec<usize>>,
    pub(crate) covariates_by_subject: HashMap<String, Vec<usize>>,
    pub(crate) covariate_types: Vec<String>,
}

impl QueryDataIndex {
    pub(crate) fn new(
        entities: &[Entity],
        reports: &[CommunityReport],
        text_units: &[TextUnit],
        relationships: &[Relationship],
        covariates: &[Covariate],
    ) -> Self {
        let mut entity_by_id = HashMap::with_capacity(entities.len());
        let mut entity_by_title = HashMap::<String, Vec<usize>>::new();
        for (index, entity) in entities.iter().enumerate() {
            entity_by_id.insert(entity.id.clone(), index);
            entity_by_title
                .entry(entity.title.clone())
                .or_default()
                .push(index);
        }

        let mut report_by_community_id = HashMap::with_capacity(reports.len());
        for (index, report) in reports.iter().enumerate() {
            report_by_community_id.insert(report.community_id.clone(), index);
        }

        let text_unit_by_id = text_units
            .iter()
            .enumerate()
            .map(|(index, unit)| (unit.id.clone(), index))
            .collect();

        let mut relationships_by_entity = HashMap::<String, Vec<usize>>::new();
        for (index, relationship) in relationships.iter().enumerate() {
            relationships_by_entity
                .entry(relationship.source.clone())
                .or_default()
                .push(index);
            if relationship.target != relationship.source {
                relationships_by_entity
                    .entry(relationship.target.clone())
                    .or_default()
                    .push(index);
            }
        }

        let mut covariates_by_subject = HashMap::<String, Vec<usize>>::new();
        let mut covariate_types = Vec::new();
        for (index, covariate) in covariates.iter().enumerate() {
            covariates_by_subject
                .entry(covariate.subject_id.clone())
                .or_default()
                .push(index);
            if !covariate_types
                .iter()
                .any(|value| value == &covariate.covariate_type)
            {
                covariate_types.push(covariate.covariate_type.clone());
            }
        }

        Self {
            entity_by_id,
            entity_by_title,
            report_by_community_id,
            text_unit_by_id,
            relationships_by_entity,
            covariates_by_subject,
            covariate_types,
        }
    }
}
