//! Graph operation domain row types.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextUnitInput {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawEntityRow {
    pub(crate) title: String,
    pub(crate) entity_type: String,
    pub(crate) description: String,
    pub(crate) source_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawRelationshipRow {
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) description: String,
    pub(crate) source_id: String,
    pub(crate) weight: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EntityRow {
    pub(crate) title: String,
    pub(crate) entity_type: String,
    pub(crate) description: Vec<String>,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) frequency: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RelationshipRow {
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) description: Vec<String>,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) weight: f64,
}

#[derive(Debug)]
pub(crate) struct ExtractedGraph {
    pub(crate) entities: Vec<EntityRow>,
    pub(crate) relationships: Vec<RelationshipRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SummarizedEntityRow {
    pub(crate) title: String,
    pub(crate) entity_type: String,
    pub(crate) description: String,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) frequency: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SummarizedRelationshipRow {
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) description: String,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) weight: f64,
}

#[derive(Debug)]
pub(crate) struct SummarizedGraph {
    pub(crate) entities: Vec<SummarizedEntityRow>,
    pub(crate) relationships: Vec<SummarizedRelationshipRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FinalEntityRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) title: String,
    pub(crate) entity_type: String,
    pub(crate) description: String,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) frequency: i64,
    pub(crate) degree: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FinalRelationshipRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) description: String,
    pub(crate) weight: f64,
    pub(crate) combined_degree: i64,
    pub(crate) text_unit_ids: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct FinalizedGraph {
    pub(crate) entities: Vec<FinalEntityRow>,
    pub(crate) relationships: Vec<FinalRelationshipRow>,
}
