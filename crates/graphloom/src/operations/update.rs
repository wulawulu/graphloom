//! Pure dataframe operations for GraphRAG-compatible incremental updates.

use std::collections::{BTreeMap, BTreeSet};

use polars_core::prelude::*;

use crate::{
    Result,
    dataframe::{
        f64_column_value, i64_column_value, list_column_at, string_list_or_string_column_at,
        string_value, usize_to_i64,
    },
    operations::graph::{EntityRow, FinalEntityRow, FinalRelationshipRow, RelationshipRow},
};

const UPDATE_CONTEXT: &str = "incremental_update";

const COMMUNITIES_FINAL_COLUMNS: &[&str] = &[
    "id",
    "human_readable_id",
    "community",
    "level",
    "parent",
    "children",
    "title",
    "entity_ids",
    "relationship_ids",
    "text_unit_ids",
    "period",
    "size",
];
const COMMUNITY_REPORTS_FINAL_COLUMNS: &[&str] = &[
    "id",
    "human_readable_id",
    "community",
    "level",
    "parent",
    "children",
    "title",
    "summary",
    "full_content",
    "rank",
    "rating_explanation",
    "findings",
    "full_content_json",
    "period",
    "size",
];

#[derive(Debug, Clone)]
pub(crate) struct MergedEntityRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) title: String,
    pub(crate) entity_type: String,
    pub(crate) descriptions: Vec<String>,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) frequency: i64,
    pub(crate) degree: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct MergedRelationshipRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) descriptions: Vec<String>,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) weight: f64,
    pub(crate) combined_degree: i64,
}

pub(crate) fn concatenate_with_rebased_ids(
    previous: &DataFrame,
    delta: &DataFrame,
) -> Result<DataFrame> {
    let mut delta = delta.clone();
    let initial_id = max_human_readable_id(previous)?
        .checked_add(1)
        .ok_or_else(|| {
            crate::dataframe::invalid_data(UPDATE_CONTEXT, "human_readable_id overflow")
        })?;
    let ids = sequential_ids(initial_id, delta.height())?;
    delta.with_column(Series::new("human_readable_id".into(), ids).into())?;
    let mut merged = previous.clone();
    merged.vstack_mut(&delta)?;
    Ok(merged)
}

pub(crate) fn merge_entities(
    previous: &DataFrame,
    delta: &DataFrame,
) -> Result<(Vec<MergedEntityRow>, BTreeMap<String, String>)> {
    let previous_rows = read_entities(previous)?;
    let mut delta_rows = read_entities(delta)?;
    let mut previous_ids = BTreeMap::<String, Vec<String>>::new();
    for row in &previous_rows {
        previous_ids
            .entry(row.title.clone())
            .or_default()
            .push(row.id.clone());
    }
    let mut id_mapping = BTreeMap::new();
    for row in &delta_rows {
        if let Some(ids) = previous_ids.get(&row.title) {
            for id in ids {
                id_mapping.insert(row.id.clone(), id.clone());
            }
        }
    }

    let initial_id = max_human_readable_id(previous)?
        .checked_add(1)
        .ok_or_else(|| {
            crate::dataframe::invalid_data(UPDATE_CONTEXT, "entity human_readable_id overflow")
        })?;
    for (index, row) in delta_rows.iter_mut().enumerate() {
        row.human_readable_id = offset_id(initial_id, index)?;
    }

    let mut grouped = BTreeMap::<String, Vec<EntityTableRow>>::new();
    for row in previous_rows.into_iter().chain(delta_rows) {
        grouped.entry(row.title.clone()).or_default().push(row);
    }
    let mut resolved = Vec::with_capacity(grouped.len());
    for (title, rows) in grouped {
        let first = rows
            .first()
            .ok_or_else(|| crate::dataframe::invalid_data(UPDATE_CONTEXT, "empty entity group"))?;
        let mut descriptions = Vec::new();
        let mut text_unit_ids = Vec::new();
        for row in &rows {
            descriptions.extend(row.descriptions.iter().cloned());
            text_unit_ids.extend(row.text_unit_ids.iter().cloned());
        }
        resolved.push(MergedEntityRow {
            id: first.id.clone(),
            human_readable_id: first.human_readable_id,
            title,
            entity_type: first.entity_type.clone(),
            descriptions,
            frequency: usize_to_i64(text_unit_ids.len(), UPDATE_CONTEXT, "frequency")?,
            text_unit_ids,
            degree: first.degree,
        });
    }
    Ok((resolved, id_mapping))
}

pub(crate) fn merge_relationships(
    previous: &DataFrame,
    delta: &DataFrame,
    entity_titles: &BTreeSet<String>,
) -> Result<Vec<MergedRelationshipRow>> {
    let previous_rows = read_relationships(previous)?;
    let mut delta_rows = read_relationships(delta)?;
    let initial_id = max_human_readable_id(previous)?
        .checked_add(1)
        .ok_or_else(|| {
            crate::dataframe::invalid_data(
                UPDATE_CONTEXT,
                "relationship human_readable_id overflow",
            )
        })?;
    for (index, row) in delta_rows.iter_mut().enumerate() {
        row.human_readable_id = offset_id(initial_id, index)?;
    }

    let mut grouped = BTreeMap::<(String, String), Vec<RelationshipTableRow>>::new();
    for row in previous_rows.into_iter().chain(delta_rows) {
        grouped
            .entry((row.source.clone(), row.target.clone()))
            .or_default()
            .push(row);
    }
    let mut merged = Vec::with_capacity(grouped.len());
    for ((source, target), rows) in grouped {
        let first = rows.first().ok_or_else(|| {
            crate::dataframe::invalid_data(UPDATE_CONTEXT, "empty relationship group")
        })?;
        let mut descriptions = Vec::new();
        let mut text_unit_ids = Vec::new();
        let mut weight = 0.0;
        for row in &rows {
            descriptions.extend(row.descriptions.iter().cloned());
            text_unit_ids.extend(row.text_unit_ids.iter().cloned());
            weight += row.weight;
        }
        let row_count = u32::try_from(rows.len()).map_err(|_| {
            crate::dataframe::invalid_data(
                UPDATE_CONTEXT,
                "relationship group exceeds the supported row count",
            )
        })?;
        let divisor = f64::from(row_count);
        merged.push(MergedRelationshipRow {
            id: first.id.clone(),
            human_readable_id: first.human_readable_id,
            source,
            target,
            descriptions,
            text_unit_ids,
            weight: weight / divisor,
            combined_degree: 0,
        });
    }

    let mut source_degrees = BTreeMap::<String, i64>::new();
    let mut target_degrees = BTreeMap::<String, i64>::new();
    for row in &merged {
        let source = source_degrees.entry(row.source.clone()).or_default();
        *source = source.saturating_add(1);
        let target = target_degrees.entry(row.target.clone()).or_default();
        *target = target.saturating_add(1);
    }
    for row in &mut merged {
        row.combined_degree = source_degrees
            .get(&row.source)
            .copied()
            .unwrap_or_default()
            .saturating_add(target_degrees.get(&row.target).copied().unwrap_or_default());
    }
    merged.retain(|row| entity_titles.contains(&row.source) && entity_titles.contains(&row.target));
    Ok(merged)
}

pub(crate) fn entity_summary_inputs(rows: &[MergedEntityRow]) -> Vec<EntityRow> {
    rows.iter()
        .map(|row| EntityRow {
            title: row.title.clone(),
            entity_type: row.entity_type.clone(),
            description: row.descriptions.clone(),
            text_unit_ids: row.text_unit_ids.clone(),
            frequency: row.frequency,
        })
        .collect()
}

pub(crate) fn relationship_summary_inputs(rows: &[MergedRelationshipRow]) -> Vec<RelationshipRow> {
    rows.iter()
        .map(|row| RelationshipRow {
            source: row.source.clone(),
            target: row.target.clone(),
            description: row.descriptions.clone(),
            text_unit_ids: row.text_unit_ids.clone(),
            weight: row.weight,
        })
        .collect()
}

pub(crate) fn summarized_entities_dataframe(
    merged: &[MergedEntityRow],
    summaries: &[crate::operations::graph::SummarizedEntityRow],
) -> Result<DataFrame> {
    let descriptions = summaries
        .iter()
        .map(|row| (row.title.as_str(), row.description.as_str()))
        .collect::<BTreeMap<_, _>>();
    let rows = merged
        .iter()
        .map(|row| FinalEntityRow {
            id: row.id.clone(),
            human_readable_id: row.human_readable_id,
            title: row.title.clone(),
            entity_type: row.entity_type.clone(),
            description: descriptions
                .get(row.title.as_str())
                .copied()
                .unwrap_or_default()
                .to_owned(),
            text_unit_ids: row.text_unit_ids.clone(),
            frequency: row.frequency,
            degree: row.degree,
        })
        .collect::<Vec<_>>();
    crate::operations::graph::final_entities_dataframe(&rows)
}

pub(crate) fn summarized_relationships_dataframe(
    merged: &[MergedRelationshipRow],
    summaries: &[crate::operations::graph::SummarizedRelationshipRow],
) -> Result<DataFrame> {
    let descriptions = summaries
        .iter()
        .map(|row| {
            (
                (row.source.as_str(), row.target.as_str()),
                row.description.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let rows = merged
        .iter()
        .map(|row| FinalRelationshipRow {
            id: row.id.clone(),
            human_readable_id: row.human_readable_id,
            source: row.source.clone(),
            target: row.target.clone(),
            description: descriptions
                .get(&(row.source.as_str(), row.target.as_str()))
                .copied()
                .unwrap_or_default()
                .to_owned(),
            weight: row.weight,
            combined_degree: row.combined_degree,
            text_unit_ids: row.text_unit_ids.clone(),
        })
        .collect::<Vec<_>>();
    crate::operations::graph::final_relationships_dataframe(&rows)
}

pub(crate) fn merge_text_units(
    previous: &DataFrame,
    delta: &DataFrame,
    entity_mapping: &BTreeMap<String, String>,
) -> Result<DataFrame> {
    let mut delta = delta.clone();
    let initial_id = max_human_readable_id(previous)?
        .checked_add(1)
        .ok_or_else(|| {
            crate::dataframe::invalid_data(UPDATE_CONTEXT, "text unit human_readable_id overflow")
        })?;
    let human_readable_ids = (0..delta.height())
        .map(|index| offset_id(initial_id, index))
        .collect::<Result<Vec<_>>>()?;
    delta.with_column(Series::new("human_readable_id".into(), human_readable_ids).into_column())?;

    let entity_ids = delta.column("entity_ids")?;
    let remapped = (0..delta.height())
        .map(|index| {
            if matches!(entity_ids.get(index)?, AnyValue::Null) {
                return Ok(None);
            }
            let values = list_column_at(&delta, index, "entity_ids", UPDATE_CONTEXT)?
                .into_iter()
                .map(|entity_id| entity_mapping.get(&entity_id).cloned().unwrap_or(entity_id))
                .collect::<Vec<_>>();
            Ok(Some(Series::new("item".into(), values)))
        })
        .collect::<Result<Vec<_>>>()?;
    delta.with_column(Series::new("entity_ids".into(), remapped).into_column())?;

    let mut merged = previous.clone();
    merged.vstack_mut(&delta)?;
    Ok(merged)
}

pub(crate) fn merge_communities(
    previous: &DataFrame,
    delta: &DataFrame,
) -> Result<(DataFrame, BTreeMap<i64, i64>)> {
    let (mut previous, mut delta) = align_optional_columns(previous, delta)?;
    let previous_ids = integer_values(&previous, "community")?;
    let old_max = previous_ids
        .into_iter()
        .map(|value| value.unwrap_or(0))
        .max()
        .unwrap_or(0);
    let mut mapping = BTreeMap::from([(-1, -1)]);
    for value in integer_values(&delta, "community")?.into_iter().flatten() {
        let mapped = value
            .checked_add(old_max)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                crate::dataframe::invalid_data(UPDATE_CONTEXT, "community ID overflow")
            })?;
        mapping.insert(value, mapped);
    }
    remap_integer_column(&mut delta, "community", &mapping)?;
    remap_integer_column(&mut delta, "parent", &mapping)?;
    cast_integer_column(&mut previous, "community")?;
    previous.vstack_mut(&delta)?;
    let communities = integer_values(&previous, "community")?
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect::<Vec<_>>();
    let titles = communities
        .iter()
        .map(|community| format!("Community {community}"))
        .collect::<Vec<_>>();
    previous.with_column(Series::new("title".into(), titles).into())?;
    previous.with_column(Series::new("human_readable_id".into(), communities).into())?;
    Ok((
        previous.select(COMMUNITIES_FINAL_COLUMNS.iter().copied())?,
        mapping,
    ))
}

pub(crate) fn merge_community_reports(
    previous: &DataFrame,
    delta: &DataFrame,
    mapping: &BTreeMap<i64, i64>,
) -> Result<DataFrame> {
    let (mut previous, mut delta) = align_optional_columns(previous, delta)?;
    remap_integer_column(&mut delta, "community", mapping)?;
    remap_integer_column(&mut delta, "parent", mapping)?;
    cast_integer_column(&mut previous, "community")?;
    previous.vstack_mut(&delta)?;
    cast_integer_column(&mut previous, "community")?;
    let communities = integer_values(&previous, "community")?
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect::<Vec<_>>();
    previous.with_column(Series::new("human_readable_id".into(), communities).into())?;
    Ok(previous.select(COMMUNITY_REPORTS_FINAL_COLUMNS.iter().copied())?)
}

fn align_optional_columns(
    previous: &DataFrame,
    delta: &DataFrame,
) -> Result<(DataFrame, DataFrame)> {
    let mut previous = previous.clone();
    let mut delta = delta.clone();
    for name in ["size", "period"] {
        let previous_dtype = previous.column(name).ok().map(Column::dtype).cloned();
        let delta_dtype = delta.column(name).ok().map(Column::dtype).cloned();
        if previous_dtype.is_none() {
            let dtype = delta_dtype.clone().unwrap_or(DataType::Null);
            previous
                .with_column(Series::full_null(name.into(), previous.height(), &dtype).into())?;
        }
        if delta_dtype.is_none() {
            let dtype = previous_dtype.unwrap_or(DataType::Null);
            delta.with_column(Series::full_null(name.into(), delta.height(), &dtype).into())?;
        }
    }
    Ok((previous, delta))
}

fn remap_integer_column(
    dataframe: &mut DataFrame,
    name: &'static str,
    mapping: &BTreeMap<i64, i64>,
) -> Result<()> {
    let values = integer_values(dataframe, name)?
        .into_iter()
        .map(|value| value.map(|value| mapping.get(&value).copied().unwrap_or(value)))
        .collect::<Vec<_>>();
    dataframe.with_column(Series::new(name.into(), values).into())?;
    Ok(())
}

fn cast_integer_column(dataframe: &mut DataFrame, name: &'static str) -> Result<()> {
    let values = integer_values(dataframe, name)?;
    dataframe.with_column(Series::new(name.into(), values).into())?;
    Ok(())
}

fn integer_values(dataframe: &DataFrame, name: &'static str) -> Result<Vec<Option<i64>>> {
    let column = dataframe.column(name)?;
    (0..dataframe.height())
        .map(|index| match column.get(index)? {
            AnyValue::Int64(value) => Ok(Some(value)),
            AnyValue::Int32(value) => Ok(Some(i64::from(value))),
            AnyValue::UInt32(value) => Ok(Some(i64::from(value))),
            AnyValue::Null => Ok(None),
            _ => Err(crate::dataframe::invalid_data(
                UPDATE_CONTEXT,
                &format!("expected integer column {name}"),
            )),
        })
        .collect()
}

fn max_human_readable_id(dataframe: &DataFrame) -> Result<i64> {
    if dataframe.height() == 0 {
        return Ok(-1);
    }
    let mut maximum = None;
    for index in 0..dataframe.height() {
        let value = i64_column_value(dataframe, index, "human_readable_id", UPDATE_CONTEXT)?;
        maximum = Some(maximum.map_or(value, |current: i64| current.max(value)));
    }
    Ok(maximum.unwrap_or(-1))
}

fn sequential_ids(initial_id: i64, length: usize) -> Result<Vec<i64>> {
    (0..length)
        .map(|index| offset_id(initial_id, index))
        .collect()
}

fn offset_id(initial_id: i64, index: usize) -> Result<i64> {
    let offset = usize_to_i64(index, UPDATE_CONTEXT, "row index")?;
    initial_id
        .checked_add(offset)
        .ok_or_else(|| crate::dataframe::invalid_data(UPDATE_CONTEXT, "human_readable_id overflow"))
}

#[derive(Debug, Clone)]
struct EntityTableRow {
    id: String,
    human_readable_id: i64,
    title: String,
    entity_type: String,
    descriptions: Vec<String>,
    text_unit_ids: Vec<String>,
    degree: i64,
}

fn read_entities(dataframe: &DataFrame) -> Result<Vec<EntityTableRow>> {
    let ids = dataframe.column("id")?.str()?;
    let titles = dataframe.column("title")?.str()?;
    let types = dataframe.column("type")?.str()?;
    (0..dataframe.height())
        .map(|index| {
            Ok(EntityTableRow {
                id: string_value(ids.get(index), "id", UPDATE_CONTEXT)?,
                human_readable_id: i64_column_value(
                    dataframe,
                    index,
                    "human_readable_id",
                    UPDATE_CONTEXT,
                )?,
                title: string_value(titles.get(index), "title", UPDATE_CONTEXT)?,
                entity_type: string_value(types.get(index), "type", UPDATE_CONTEXT)?,
                descriptions: string_list_or_string_column_at(
                    dataframe,
                    index,
                    "description",
                    UPDATE_CONTEXT,
                )?,
                text_unit_ids: list_column_at(dataframe, index, "text_unit_ids", UPDATE_CONTEXT)?,
                degree: i64_column_value(dataframe, index, "degree", UPDATE_CONTEXT)?,
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
struct RelationshipTableRow {
    id: String,
    human_readable_id: i64,
    source: String,
    target: String,
    descriptions: Vec<String>,
    text_unit_ids: Vec<String>,
    weight: f64,
}

fn read_relationships(dataframe: &DataFrame) -> Result<Vec<RelationshipTableRow>> {
    let ids = dataframe.column("id")?.str()?;
    let sources = dataframe.column("source")?.str()?;
    let targets = dataframe.column("target")?.str()?;
    (0..dataframe.height())
        .map(|index| {
            Ok(RelationshipTableRow {
                id: string_value(ids.get(index), "id", UPDATE_CONTEXT)?,
                human_readable_id: i64_column_value(
                    dataframe,
                    index,
                    "human_readable_id",
                    UPDATE_CONTEXT,
                )?,
                source: string_value(sources.get(index), "source", UPDATE_CONTEXT)?,
                target: string_value(targets.get(index), "target", UPDATE_CONTEXT)?,
                descriptions: string_list_or_string_column_at(
                    dataframe,
                    index,
                    "description",
                    UPDATE_CONTEXT,
                )?,
                text_unit_ids: list_column_at(dataframe, index, "text_unit_ids", UPDATE_CONTEXT)?,
                weight: f64_column_value(dataframe, index, "weight", UPDATE_CONTEXT)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dataframe::{i64_list_column, list_column},
        operations::{
            graph::{FinalEntityRow, FinalRelationshipRow},
            text_units::{TextUnitRow, text_units_dataframe},
        },
    };

    macro_rules! entity {
        (
            $id:expr,
            $human_id:expr,
            $title:expr,
            $entity_type:expr,
            $description:expr,
            $text_units:expr,
            $frequency:expr,
            $degree:expr
            $(,)?
        ) => {
            FinalEntityRow {
                id: $id.to_owned(),
                human_readable_id: $human_id,
                title: $title.to_owned(),
                entity_type: $entity_type.to_owned(),
                description: $description.to_owned(),
                text_unit_ids: $text_units
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
                frequency: $frequency,
                degree: $degree,
            }
        };
    }

    macro_rules! relationship {
        (
            $id:expr,
            $human_id:expr,
            $source:expr,
            $target:expr,
            $description:expr,
            $weight:expr,
            $combined_degree:expr,
            $text_units:expr
            $(,)?
        ) => {
            FinalRelationshipRow {
                id: $id.to_owned(),
                human_readable_id: $human_id,
                source: $source.to_owned(),
                target: $target.to_owned(),
                description: $description.to_owned(),
                weight: $weight,
                combined_degree: $combined_degree,
                text_unit_ids: $text_units
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            }
        };
    }

    #[test]
    fn test_should_rebase_delta_ids_and_preserve_concat_order() {
        let previous = df!(
            "id" => ["old-a", "old-b"],
            "human_readable_id" => [3_i64, 8],
            "value" => ["a", "b"],
        )
        .expect("previous");
        let delta = df!(
            "id" => ["new-a", "new-b"],
            "human_readable_id" => [0_i64, 1],
            "value" => ["c", "d"],
        )
        .expect("delta");

        let merged = concatenate_with_rebased_ids(&previous, &delta).expect("merge");

        assert_eq!(
            strings(&merged, "id"),
            vec!["old-a", "old-b", "new-a", "new-b"]
        );
        assert_eq!(integers(&merged, "human_readable_id"), vec![3, 8, 9, 10]);
        assert_eq!(
            merged
                .get_column_names()
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "human_readable_id", "value"]
        );
    }

    #[test]
    fn test_should_merge_entities_by_sorted_title_and_keep_first_identity_fields() {
        let previous = crate::operations::graph::final_entities_dataframe(&[
            entity!("old-z", 5, "ZED", "person", "old z", &["tu-z"], 1, 7),
            entity!(
                "old-a",
                6,
                "ALPHA",
                "organization",
                "old a",
                &["tu-a"],
                1,
                4,
            ),
        ])
        .expect("previous entities");
        let delta = crate::operations::graph::final_entities_dataframe(&[
            entity!("new-a", 0, "ALPHA", "person", "new a", &["tu-new"], 1, 99),
            entity!("new-b", 1, "BETA", "geo", "new b", &["tu-b"], 1, 2),
        ])
        .expect("delta entities");

        let (merged, mapping) = merge_entities(&previous, &delta).expect("entity merge");

        assert_eq!(
            merged
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            vec!["ALPHA", "BETA", "ZED"]
        );
        let alpha = merged.first().expect("alpha");
        assert_eq!(alpha.id, "old-a");
        assert_eq!(alpha.entity_type, "organization");
        assert_eq!(alpha.human_readable_id, 6);
        assert_eq!(alpha.descriptions, vec!["old a", "new a"]);
        assert_eq!(alpha.text_unit_ids, vec!["tu-a", "tu-new"]);
        assert_eq!(alpha.frequency, 2);
        assert_eq!(alpha.degree, 4);
        assert_eq!(mapping.get("new-a").map(String::as_str), Some("old-a"));
        assert!(!mapping.contains_key("new-b"));
        assert_eq!(merged[1].human_readable_id, 8);
    }

    #[test]
    fn test_should_merge_relationships_sort_groups_recompute_degrees_and_filter_orphans() {
        let previous = crate::operations::graph::final_relationships_dataframe(&[
            relationship!("old-ab", 4, "A", "B", "old", 2.0, 99, &["tu-old"]),
            relationship!("old-orphan", 5, "A", "MISSING", "orphan", 1.0, 1, &["tu-o"]),
        ])
        .expect("previous relationships");
        let delta = crate::operations::graph::final_relationships_dataframe(&[
            relationship!("new-ab", 0, "A", "B", "new", 4.0, 50, &["tu-new"]),
            relationship!("new-bc", 1, "B", "C", "bc", 5.0, 50, &["tu-bc"]),
        ])
        .expect("delta relationships");
        let titles = ["A", "B", "C"].into_iter().map(str::to_owned).collect();

        let merged = merge_relationships(&previous, &delta, &titles).expect("relationship merge");

        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged
                .iter()
                .map(|row| (row.source.as_str(), row.target.as_str()))
                .collect::<Vec<_>>(),
            vec![("A", "B"), ("B", "C")]
        );
        assert_eq!(merged[0].id, "old-ab");
        assert_eq!(merged[0].human_readable_id, 4);
        assert_eq!(merged[0].descriptions, vec!["old", "new"]);
        assert_eq!(merged[0].text_unit_ids, vec!["tu-old", "tu-new"]);
        assert!((merged[0].weight - 3.0).abs() < f64::EPSILON);
        assert_eq!(merged[0].combined_degree, 3);
        assert_eq!(merged[1].combined_degree, 2);
    }

    #[test]
    fn test_should_remap_text_unit_entities_and_rebase_ids() {
        let previous = text_units_dataframe(&[text_unit("old-tu", 7, &["old-entity"])])
            .expect("previous text units");
        let delta = text_units_dataframe(&[text_unit("new-tu", 0, &["delta-mapped", "delta-new"])])
            .expect("delta text units");
        let mapping = BTreeMap::from([("delta-mapped".to_owned(), "old-entity".to_owned())]);

        let merged = merge_text_units(&previous, &delta, &mapping).expect("text unit merge");

        assert_eq!(strings(&merged, "id"), vec!["old-tu", "new-tu"]);
        assert_eq!(integers(&merged, "human_readable_id"), vec![7, 8]);
        assert_eq!(
            list_column_at(&merged, 1, "entity_ids", UPDATE_CONTEXT).expect("entity ids"),
            vec!["old-entity", "delta-new"]
        );
        assert_eq!(
            merged.column("document_id").expect("document_id").dtype(),
            &DataType::String
        );
    }

    #[test]
    fn test_should_preserve_null_text_unit_entity_ids() {
        let previous = text_units_dataframe(&[text_unit("old-tu", 7, &["old-entity"])])
            .expect("previous text units");
        let mut delta =
            text_units_dataframe(&[text_unit("new-tu", 0, &[])]).expect("delta text units");
        let null_entity_ids = Series::new("entity_ids".into(), vec![Option::<Series>::None]);
        delta
            .with_column(null_entity_ids.into_column())
            .expect("null entity ids");

        let merged =
            merge_text_units(&previous, &delta, &BTreeMap::new()).expect("text unit merge");

        assert!(matches!(
            merged
                .column("entity_ids")
                .expect("entity_ids")
                .get(1)
                .expect("row"),
            AnyValue::Null
        ));
    }

    #[test]
    fn test_should_treat_null_previous_community_as_zero_for_rebasing() {
        let mut previous = community_dataframe("old-community", 0, -1, &[0], "old title");
        previous
            .with_column(Series::new("community".into(), vec![Option::<i64>::None]).into_column())
            .expect("nullable community");
        let delta = community_dataframe("new-community", 0, -1, &[0], "delta title");

        let (communities, mapping) =
            merge_communities(&previous, &delta).expect("communities merge");

        assert_eq!(mapping.get(&0), Some(&1));
        assert_eq!(integers(&communities, "community"), vec![0, 1]);
    }

    #[test]
    fn test_should_leave_community_children_and_report_title_unmapped() {
        let previous = community_dataframe("old-community", 0, -1, &[0], "old title");
        let delta = community_dataframe("new-community", 0, -1, &[0, 1], "delta title");
        let (communities, mapping) =
            merge_communities(&previous, &delta).expect("communities merge");
        assert_eq!(integers(&communities, "community"), vec![0, 1]);
        assert_eq!(integers(&communities, "parent"), vec![-1, -1]);
        assert_eq!(
            crate::dataframe::i64_list_column_at(&communities, 1, "children", UPDATE_CONTEXT,)
                .expect("children"),
            vec![0, 1]
        );
        assert_eq!(
            strings(&communities, "title"),
            vec!["Community 0", "Community 1"]
        );

        let previous_reports = report_dataframe("old-report", 0, -1, &[0], "old report");
        let delta_reports = report_dataframe("new-report", 0, -1, &[0, 1], "delta report title");
        let reports = merge_community_reports(&previous_reports, &delta_reports, &mapping)
            .expect("report merge");
        assert_eq!(integers(&reports, "community"), vec![0, 1]);
        assert_eq!(
            strings(&reports, "title"),
            vec!["old report", "delta report title"]
        );
        assert_eq!(
            crate::dataframe::i64_list_column_at(&reports, 1, "children", UPDATE_CONTEXT)
                .expect("report children"),
            vec![0, 1]
        );
    }

    fn text_unit(id: &str, human_readable_id: i64, entity_ids: &[&str]) -> TextUnitRow {
        TextUnitRow {
            id: id.to_owned(),
            human_readable_id,
            text: id.to_owned(),
            n_tokens: 1,
            document_id: format!("doc-{id}"),
            entity_ids: entity_ids.iter().map(|value| (*value).to_owned()).collect(),
            relationship_ids: Vec::new(),
            covariate_ids: Vec::new(),
        }
    }

    fn community_dataframe(
        id: &str,
        community: i64,
        parent: i64,
        children: &[i64],
        title: &str,
    ) -> DataFrame {
        let mut dataframe = df!(
            "id" => [id],
            "human_readable_id" => [community],
            "community" => [community],
            "level" => [0_i64],
            "parent" => [parent],
            "title" => [title],
            "period" => ["2026-01-01"],
            "size" => [1_i64],
        )
        .expect("community dataframe");
        dataframe
            .insert_column(5, i64_list_column("children", &[children.to_vec()]))
            .expect("children");
        for (index, name) in [
            (7, "entity_ids"),
            (8, "relationship_ids"),
            (9, "text_unit_ids"),
        ] {
            dataframe
                .insert_column(index, list_column(name, &[vec![format!("{name}-1")]]))
                .expect("list column");
        }
        dataframe
    }

    fn report_dataframe(
        id: &str,
        community: i64,
        parent: i64,
        children: &[i64],
        title: &str,
    ) -> DataFrame {
        let mut dataframe = df!(
            "id" => [id],
            "human_readable_id" => [community],
            "community" => [community],
            "level" => [0_i64],
            "parent" => [parent],
            "title" => [title],
            "summary" => ["summary"],
            "full_content" => ["content"],
            "rank" => [5.0_f64],
            "rating_explanation" => ["rating"],
            "findings" => ["findings"],
            "full_content_json" => ["{}"],
            "period" => ["2026-01-01"],
            "size" => [1_i64],
        )
        .expect("report dataframe");
        dataframe
            .insert_column(5, i64_list_column("children", &[children.to_vec()]))
            .expect("children");
        dataframe
    }

    fn strings(dataframe: &DataFrame, column: &str) -> Vec<String> {
        let values = dataframe
            .column(column)
            .expect("column")
            .str()
            .expect("string");
        (0..dataframe.height())
            .filter_map(|index| values.get(index).map(str::to_owned))
            .collect()
    }

    fn integers(dataframe: &DataFrame, column: &str) -> Vec<i64> {
        let values = dataframe
            .column(column)
            .expect("column")
            .i64()
            .expect("i64");
        values.into_no_null_iter().collect()
    }
}
