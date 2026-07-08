//! Graph extraction, summarization, finalization, table, and snapshot operations.

mod extraction;
mod finalize;
mod merge;
mod snapshot;
mod summarize;
mod tables;
mod types;

pub(crate) use extraction::extract_text_unit_graph;
pub(crate) use finalize::{degree_map, finalize_entities, finalize_relationships};
pub(crate) use merge::{filter_orphan_relationships, merge_entities, merge_relationships};
pub(crate) use snapshot::graphml_snapshot;
pub(crate) use summarize::{
    DescriptionSummarizeConfig, summarize_entities, summarize_relationships,
};
pub(crate) use tables::{
    entity_intermediate_dataframe, extract_graph_sample, final_entities_dataframe,
    final_relationships_dataframe, finalize_graph_sample, raw_entity_dataframe,
    raw_relationship_dataframe, read_entity_rows, read_relationship_rows, read_text_units,
    relationship_intermediate_dataframe,
};
pub(crate) use types::{
    EntityRow, FinalEntityRow, FinalRelationshipRow, RawEntityRow, RawRelationshipRow,
    RelationshipRow, SummarizedEntityRow, SummarizedRelationshipRow, TextUnitInput,
};
