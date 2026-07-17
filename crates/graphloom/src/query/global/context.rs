//! Fixed-selection Global Search community weighting and batching.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use graphloom_llm::Tokenizer;
use polars_core::prelude::{Column, DataFrame, NamedFrom, Series};

use super::{DynamicRating, random::PythonRandom};
use crate::{
    config::GlobalSearchConfig,
    query::{
        CommunityReport, GlobalQueryData, QueryContextRecords, QueryContextText,
        QueryUsageCategory, Result, SearchMethod, context::ContextTable,
    },
};

const WEIGHT_COLUMN: &str = "occurrence weight";
const RANK_COLUMN: &str = "rank";
const CONTEXT_NAME: &str = "Reports";

#[derive(Debug, Clone)]
pub(crate) struct GlobalContextResult {
    pub(crate) batches: Vec<String>,
    pub(crate) records: Vec<DataFrame>,
    pub(crate) usage: QueryUsageCategory,
    pub(crate) dynamic_ratings: Vec<DynamicRating>,
}

impl GlobalContextResult {
    pub(crate) fn context_text(&self) -> QueryContextText {
        QueryContextText::Batches(self.batches.clone())
    }

    pub(crate) fn context_records(&self) -> QueryContextRecords {
        QueryContextRecords::Batches(self.records.clone())
    }
}

#[derive(Debug)]
pub(crate) struct GlobalContextBuilder {
    pub(super) config: GlobalSearchConfig,
    pub(super) entities: Vec<crate::query::Entity>,
    pub(super) communities: Vec<crate::query::Community>,
    pub(super) reports: Vec<CommunityReport>,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
}

#[derive(Debug, Clone)]
struct WeightedReport {
    report: CommunityReport,
    occurrence_weight: f64,
}

impl GlobalContextBuilder {
    pub(crate) fn new(
        config: GlobalSearchConfig,
        data: GlobalQueryData,
        tokenizer: Arc<dyn Tokenizer>,
    ) -> Self {
        Self {
            config,
            entities: data.entities,
            communities: data.communities,
            reports: data.reports,
            tokenizer,
        }
    }

    pub(crate) fn build_fixed(&self) -> Result<GlobalContextResult> {
        self.build_selected(
            self.reports.clone(),
            QueryUsageCategory::default(),
            Vec::new(),
        )
    }

    pub(crate) fn build_selected(
        &self,
        reports: Vec<CommunityReport>,
        usage: QueryUsageCategory,
        dynamic_ratings: Vec<DynamicRating>,
    ) -> Result<GlobalContextResult> {
        let weights = self.community_weights();
        let max_weight = reports
            .iter()
            .filter_map(|report| weights.get(&report.community_id))
            .copied()
            .max()
            .unwrap_or(0);
        let mut selected = reports
            .into_iter()
            .filter(|report| report.rank.is_some_and(|rank| rank >= 0.0))
            .map(|report| {
                let raw = weights.get(&report.community_id).copied().unwrap_or(0);
                let occurrence_weight = if max_weight == 0 {
                    // GraphRAG divides by zero here. A finite zero preserves ordering and
                    // avoids leaking NaN into provider prompts for an otherwise valid index.
                    0.0
                } else {
                    raw as f64 / max_weight as f64
                };
                WeightedReport {
                    report,
                    occurrence_weight,
                }
            })
            .collect::<Vec<_>>();
        PythonRandom::new(86).shuffle(&mut selected);
        self.batch(selected, usage, dynamic_ratings)
    }

    pub(crate) const fn config(&self) -> &GlobalSearchConfig {
        &self.config
    }

    fn community_weights(&self) -> HashMap<String, usize> {
        let mut text_units = HashMap::<String, HashSet<String>>::new();
        for entity in &self.entities {
            for community_id in &entity.community_ids {
                text_units
                    .entry(community_id.clone())
                    .or_default()
                    .extend(entity.text_unit_ids.iter().cloned());
            }
        }
        self.reports
            .iter()
            .map(|report| {
                (
                    report.community_id.clone(),
                    text_units.get(&report.community_id).map_or(0, HashSet::len),
                )
            })
            .collect()
    }

    fn batch(
        &self,
        reports: Vec<WeightedReport>,
        usage: QueryUsageCategory,
        dynamic_ratings: Vec<DynamicRating>,
    ) -> Result<GlobalContextResult> {
        if reports.is_empty() {
            return Ok(GlobalContextResult {
                batches: Vec::new(),
                records: Vec::new(),
                usage,
                dynamic_ratings,
            });
        }
        let header = ["id", "title", WEIGHT_COLUMN, "content", RANK_COLUMN];
        let header_table = ContextTable::new(header, Vec::new());
        let initial = header_table.render_csv_header(
            CONTEXT_NAME,
            SearchMethod::Global,
            "render Global community header",
        )?;
        let initial_tokens = self.count(&initial, "count Global community header tokens")?;
        let mut current = Vec::<WeightedReport>::new();
        let mut current_tokens = initial_tokens;
        let mut batches = Vec::new();
        let mut records = Vec::new();

        for report in reports {
            let row = report_row(&report);
            let row_text = header_table.render_csv_row(
                &row,
                SearchMethod::Global,
                "render Global community row",
            )?;
            let row_tokens = self.count(&row_text, "count Global community row tokens")?;
            if current_tokens.saturating_add(row_tokens) > self.config.max_context_tokens {
                cut_batch(&mut current, &mut batches, &mut records)?;
                current_tokens = initial_tokens;
            }
            current.push(report);
            current_tokens = current_tokens.saturating_add(row_tokens);
        }
        cut_batch(&mut current, &mut batches, &mut records)?;
        Ok(GlobalContextResult {
            batches,
            records,
            usage,
            dynamic_ratings,
        })
    }

    fn count(&self, text: &str, operation: &'static str) -> Result<usize> {
        self.tokenizer
            .count(text)
            .map_err(|source| crate::query::QueryError::QueryContext {
                method: SearchMethod::Global,
                operation,
                message: source.to_string(),
            })
    }
}

fn cut_batch(
    current: &mut Vec<WeightedReport>,
    batches: &mut Vec<String>,
    records: &mut Vec<DataFrame>,
) -> Result<()> {
    if current.is_empty() {
        return Ok(());
    }
    current.sort_by(|left, right| {
        right
            .occurrence_weight
            .partial_cmp(&left.occurrence_weight)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                right
                    .report
                    .rank
                    .partial_cmp(&left.report.rank)
                    .unwrap_or(Ordering::Equal)
            })
    });
    let table = ContextTable::new(
        ["id", "title", WEIGHT_COLUMN, "content", RANK_COLUMN],
        current.iter().map(report_row).collect(),
    );
    batches.push(table.render_csv(SearchMethod::Global, "render Global community batch")?);
    records.push(report_dataframe(current)?);
    current.clear();
    Ok(())
}

fn report_dataframe(reports: &[WeightedReport]) -> Result<DataFrame> {
    let columns = Vec::<Column>::from([
        Series::new(
            "id".into(),
            reports
                .iter()
                .map(|value| value.report.short_id.as_str())
                .collect::<Vec<_>>(),
        )
        .into(),
        Series::new(
            "title".into(),
            reports
                .iter()
                .map(|value| value.report.title.as_str())
                .collect::<Vec<_>>(),
        )
        .into(),
        Series::new(
            WEIGHT_COLUMN.into(),
            reports
                .iter()
                .map(|value| value.occurrence_weight)
                .collect::<Vec<_>>(),
        )
        .into(),
        Series::new(
            "content".into(),
            reports
                .iter()
                .map(|value| value.report.full_content.as_str())
                .collect::<Vec<_>>(),
        )
        .into(),
        Series::new(
            RANK_COLUMN.into(),
            reports
                .iter()
                .map(|value| value.report.rank.unwrap_or_default())
                .collect::<Vec<_>>(),
        )
        .into(),
    ]);
    DataFrame::new(reports.len(), columns).map_err(|source| {
        crate::query::QueryError::QueryContext {
            method: SearchMethod::Global,
            operation: "build Global community batch records",
            message: source.to_string(),
        }
    })
}

fn report_row(value: &WeightedReport) -> Vec<String> {
    vec![
        value.report.short_id.clone(),
        value.report.title.clone(),
        python_float(value.occurrence_weight),
        value.report.full_content.clone(),
        python_float(value.report.rank.unwrap_or_default()),
    ]
}

fn python_float(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let absolute = value.abs();
    if absolute != 0.0 && !(1.0e-4..1.0e16).contains(&absolute) {
        let scientific = format!("{value:e}");
        let Some((mantissa, exponent)) = scientific.split_once('e') else {
            return scientific;
        };
        let Ok(parsed_exponent) = exponent.parse::<i32>() else {
            return scientific;
        };
        return format!("{mantissa}e{parsed_exponent:+03}");
    }
    let rendered = value.to_string();
    if value.fract() == 0.0 {
        format!("{rendered}.0")
    } else {
        rendered
    }
}

pub(crate) fn global_context(
    map: &GlobalContextResult,
    reduce: String,
    map_outputs: DataFrame,
) -> Result<crate::query::QueryContext> {
    let reduce_records = ContextTable::new(["report_data"], vec![vec![reduce.clone()]])
        .to_dataframe(SearchMethod::Global, "build Global reduce context records")?;
    let (dynamic_text, dynamic_records) = dynamic_context(&map.dynamic_ratings)?;
    Ok(crate::query::QueryContext {
        text: QueryContextText::Composite(BTreeMap::from([
            ("dynamic".to_owned(), dynamic_text),
            ("map".to_owned(), map.context_text()),
            ("reduce".to_owned(), QueryContextText::Text(reduce)),
        ])),
        records: QueryContextRecords::Named(BTreeMap::from([
            ("dynamic".to_owned(), dynamic_records),
            ("map".to_owned(), map.context_records()),
            (
                "map_outputs".to_owned(),
                QueryContextRecords::Batches(vec![map_outputs]),
            ),
            (
                "reduce".to_owned(),
                QueryContextRecords::Batches(vec![reduce_records]),
            ),
        ])),
    })
}

fn dynamic_context(ratings: &[DynamicRating]) -> Result<(QueryContextText, QueryContextRecords)> {
    if ratings.is_empty() {
        return Ok((QueryContextText::Empty, QueryContextRecords::Empty));
    }
    let rows = ratings
        .iter()
        .map(|rating| {
            vec![
                rating.community_id.clone(),
                rating.selected_rating.to_string(),
                rating
                    .repeated_ratings
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                rating.selected.to_string(),
                rating.level.to_string(),
            ]
        })
        .collect::<Vec<_>>();
    let records = ContextTable::new(
        [
            "community_id",
            "selected_rating",
            "repeated_ratings",
            "selected",
            "level",
        ],
        rows,
    )
    .to_dataframe(
        SearchMethod::Global,
        "build Dynamic Community Selection records",
    )?;
    let text = ratings
        .iter()
        .map(|rating| {
            (
                rating.community_id.clone(),
                format!(
                    "rating={}; repeats={:?}; selected={}; level={}",
                    rating.selected_rating, rating.repeated_ratings, rating.selected, rating.level
                ),
            )
        })
        .collect();
    Ok((
        QueryContextText::Named(text),
        QueryContextRecords::Batches(vec![records]),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use graphloom_llm::{LlmError, Tokenizer};
    use polars_core::prelude::DataType;

    use super::GlobalContextBuilder;
    use crate::{
        config::GlobalSearchConfig,
        query::{CommunityReport, Entity, GlobalQueryData},
    };

    #[derive(Debug)]
    struct ByteTokenizer;

    impl Tokenizer for ByteTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text.bytes().map(u32::from).collect())
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            tokens
                .iter()
                .map(|value| {
                    u8::try_from(*value)
                        .map(char::from)
                        .map_err(|source| LlmError::Tokenizer {
                            encoding_model: "byte-test".to_owned(),
                            message: source.to_string(),
                        })
                })
                .collect()
        }
    }

    fn entity(communities: &[&str], text_units: &[&str]) -> Entity {
        Entity {
            id: "entity".to_owned(),
            short_id: None,
            title: "Entity".to_owned(),
            entity_type: None,
            description: None,
            community_ids: communities.iter().map(ToString::to_string).collect(),
            text_unit_ids: text_units.iter().map(ToString::to_string).collect(),
            rank: None,
        }
    }

    fn report(id: usize, community_id: &str, rank: Option<f64>) -> CommunityReport {
        CommunityReport {
            id: format!("report-{id}"),
            short_id: id.to_string(),
            community_id: community_id.to_owned(),
            title: format!("Report {id}"),
            summary: format!("Summary {id}"),
            full_content: format!("Full content {id}"),
            rank,
            full_content_embedding: None,
        }
    }

    fn builder(
        config: GlobalSearchConfig,
        entities: Vec<Entity>,
        reports: Vec<CommunityReport>,
    ) -> GlobalContextBuilder {
        GlobalContextBuilder::new(
            config,
            GlobalQueryData {
                entities,
                communities: Vec::new(),
                reports,
            },
            Arc::new(ByteTokenizer),
        )
    }

    #[test]
    fn test_should_dedupe_occurrences_normalize_filter_rank_and_use_full_content() {
        let result = builder(
            GlobalSearchConfig::default(),
            vec![
                entity(&["a"], &["x", "x", "y"]),
                entity(&["a", "b"], &["y", "z"]),
            ],
            vec![
                report(0, "a", Some(5.0)),
                report(1, "b", Some(8.0)),
                report(2, "c", None),
            ],
        )
        .build_fixed()
        .expect("context");
        assert_eq!(result.batches.len(), 1);
        assert_eq!(
            result.records[0]
                .column("occurrence weight")
                .expect("weight")
                .dtype(),
            &DataType::Float64
        );
        assert_eq!(
            result.records[0].column("rank").expect("rank").dtype(),
            &DataType::Float64
        );
        assert_eq!(
            result.batches[0],
            "id|title|occurrence weight|content|rank\n0|Report 0|1.0|Full content 0|5.0\n1|Report \
             1|0.6666666666666666|Full content 1|8.0\n"
        );
    }

    #[test]
    fn test_should_use_finite_zero_when_all_occurrence_weights_are_zero() {
        let result = builder(
            GlobalSearchConfig::default(),
            vec![entity(&["other"], &[])],
            vec![report(0, "a", Some(1.0))],
        )
        .build_fixed()
        .expect("zero context");
        assert!(result.batches[0].contains("|0.0|"));
        assert!(!result.batches[0].contains("NaN"));
    }

    #[test]
    fn test_should_preserve_python_shuffle_batch_membership_and_sort_inside_batch() {
        let header_tokens = "-----Reports-----\nid|title|occurrence weight|content|rank\n".len();
        let row_tokens = "0|Report 0|1.0|Full content 0|0.0\n".len();
        let config = GlobalSearchConfig {
            max_context_tokens: header_tokens + row_tokens * 2,
            ..GlobalSearchConfig::default()
        };
        let reports = (0..4)
            .map(|id| report(id, &id.to_string(), Some(id as f64)))
            .collect();
        let entities = (0..4)
            .map(|id| entity(&[&id.to_string()], &["unit"]))
            .collect();
        let result = builder(config, entities, reports)
            .build_fixed()
            .expect("batches");
        let golden: Vec<String> = serde_json::from_str(include_str!(
            "../../../../../tests/compat/fixtures/query/global_batches.json"
        ))
        .expect("shared GraphRAG golden");
        assert_eq!(result.batches, golden);
        let ids = result
            .records
            .iter()
            .map(|frame| {
                frame
                    .column("id")
                    .expect("id")
                    .str()
                    .expect("string")
                    .iter()
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![vec!["3", "1"], vec!["2", "0"]]);
    }

    #[test]
    fn test_should_keep_oversized_single_record_in_its_own_batch() {
        let config = GlobalSearchConfig {
            max_context_tokens: 1,
            ..GlobalSearchConfig::default()
        };
        let result = builder(
            config,
            vec![entity(&["a"], &["x"])],
            vec![report(0, "a", Some(1.0))],
        )
        .build_fixed()
        .expect("oversized row");
        assert_eq!(result.batches.len(), 1);
        assert!(result.batches[0].contains("Full content 0"));
    }

    #[test]
    fn test_should_render_python_float_thresholds_and_exponents() {
        assert_eq!(super::python_float(1.0), "1.0");
        assert_eq!(super::python_float(-0.0), "-0.0");
        assert_eq!(super::python_float(0.0001), "0.0001");
        assert_eq!(super::python_float(0.00001), "1e-05");
        assert_eq!(super::python_float(1.0e16), "1e+16");
    }
}
