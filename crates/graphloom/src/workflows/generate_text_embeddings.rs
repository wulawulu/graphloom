//! Text embedding generation workflow.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use futures_util::StreamExt;
use graphloom_cache::Cache;
use graphloom_llm::TiktokenTokenizer;
use graphloom_storage::{Table, TableProvider};
use graphloom_vectors::{VectorDocument, VectorIndexSchema, VectorStore};
use polars_core::{frame::row::Row, prelude::*};
use serde_json::{Value, json};

use super::common::{resolve_embedding_encoding_model, resolve_embedding_model};
use crate::{
    COMMUNITY_FULL_CONTENT_EMBEDDING, ENTITY_DESCRIPTION_EMBEDDING, GraphLoomError, GraphRagConfig,
    PipelineRunContext, Result, TEXT_UNIT_TEXT_EMBEDDING, Workflow, WorkflowFunctionOutput,
    dataframe::invalid_data,
    operations::embeddings::{EmbeddingOperationConfig, EmbeddingSourceRow, embed_text_rows},
};

/// Workflow name.
pub const GENERATE_TEXT_EMBEDDINGS_WORKFLOW: &str = "generate_text_embeddings";
const CHUNK_OVERLAP: usize = 100;

/// Generate and store text embeddings.
#[derive(Debug, Clone, Copy, Default)]
pub struct GenerateTextEmbeddingsWorkflow;

#[derive(Debug, Clone, Copy)]
struct EmbeddingField {
    name: &'static str,
    source_table: &'static str,
    id_column: &'static str,
    text_columns: &'static [&'static str],
    mapper: TextMapper,
}

#[derive(Debug, Clone, Copy)]
enum TextMapper {
    Single(&'static str),
    EntityDescription,
}

#[derive(Debug, Clone)]
struct FieldSummary {
    name: String,
    source_table: String,
    input_rows: usize,
    embedded_rows: usize,
    skipped_rows: usize,
    snippet_count: usize,
    request_count: usize,
    index_name: String,
}

#[derive(Debug, Clone)]
struct FieldDependencies {
    vector_store: Arc<dyn VectorStore>,
    model: Arc<dyn graphloom_llm::EmbeddingModel>,
    tokenizer: Arc<dyn graphloom_llm::Tokenizer>,
    embedding_cache: Option<Arc<dyn Cache>>,
    snapshot_provider: Option<Arc<dyn TableProvider>>,
}

struct FieldInput<'a> {
    source: &'a mut dyn Table,
    indices: &'a SourceIndices,
    snapshot: &'a mut Option<Box<dyn Table>>,
    input_rows: usize,
}

struct FlushState<'a> {
    buffer: &'a mut Vec<EmbeddingSourceRow>,
    snapshot: &'a mut Option<Box<dyn Table>>,
    summary: &'a mut FieldSummary,
    completed: &'a mut usize,
    total: usize,
}

impl FieldSummary {
    fn value(&self) -> Value {
        json!({
            "name": self.name,
            "source_table": self.source_table,
            "input_rows": self.input_rows,
            "embedded_rows": self.embedded_rows,
            "skipped_rows": self.skipped_rows,
            "snippet_count": self.snippet_count,
            "request_count": self.request_count,
            "index_name": self.index_name,
        })
    }
}

#[async_trait]
impl Workflow for GenerateTextEmbeddingsWorkflow {
    fn name(&self) -> &'static str {
        GENERATE_TEXT_EMBEDDINGS_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        config
            .validate_embed_text()
            .map_err(|message| GraphLoomError::InvalidData {
                workflow: GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                message,
            })?;

        let model = resolve_embedding_model(
            context,
            &config.embed_text.embedding_model_id,
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
        )?;
        let encoding_model =
            resolve_embedding_encoding_model(config, &config.embed_text.embedding_model_id);
        let tokenizer: Arc<dyn graphloom_llm::Tokenizer> =
            Arc::new(TiktokenTokenizer::new(encoding_model)?);
        let vector_store = context.vector_store();
        let embedding_cache = context
            .cache()
            .map(|cache| cache.child(&config.embed_text.model_instance_name))
            .transpose()?;

        let field_map = embedding_fields();
        let snapshot_provider = if config.snapshots.embeddings {
            Some(context.output_table_provider().child(Some("embeddings"))?)
        } else {
            None
        };
        let dependencies = FieldDependencies {
            vector_store,
            model,
            tokenizer,
            embedding_cache,
            snapshot_provider,
        };
        let mut summaries = Vec::new();
        let mut input_rows = 0usize;
        let mut output_rows = 0usize;

        for embedding_name in &config.embed_text.names {
            let field = field_map.get(embedding_name.as_str()).ok_or_else(|| {
                GraphLoomError::InvalidData {
                    workflow: GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                    message: format!("unsupported embedding name {embedding_name}"),
                }
            })?;
            if !context
                .output_table_provider()
                .has(field.source_table)
                .await?
            {
                context.callbacks.warning(
                    GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                    &format!(
                        "embedding {} source table {} is missing; skipping",
                        field.name, field.source_table
                    ),
                );
                continue;
            }

            let schema = config.vector_store.schema_for(field.name);
            dependencies.vector_store.ensure_index(&schema).await?;
            let summary = process_field(config, context, field, &schema, &dependencies).await?;
            input_rows = input_rows.saturating_add(summary.input_rows);
            output_rows = output_rows.saturating_add(summary.embedded_rows);
            summaries.push(summary);
        }

        context.stats.embedding_count = context.stats.embedding_count.saturating_add(output_rows);
        Ok(WorkflowFunctionOutput {
            result: summaries.iter().map(FieldSummary::value).collect(),
            stop: false,
            input_rows,
            output_rows,
        })
    }
}

async fn process_field(
    config: &GraphRagConfig,
    context: &mut PipelineRunContext,
    field: &EmbeddingField,
    schema: &VectorIndexSchema,
    dependencies: &FieldDependencies,
) -> Result<FieldSummary> {
    let mut source = context
        .output_table_provider()
        .open(field.source_table, false)
        .await?;
    let input_rows = source.len();
    let columns = source.column_names();
    let indices = SourceIndices::new(field, &columns)?;
    let mut snapshot = match &dependencies.snapshot_provider {
        Some(provider) => Some(provider.open(field.name, true).await?),
        None => None,
    };

    let result = process_field_inner(
        config,
        context,
        field,
        schema,
        dependencies,
        FieldInput {
            source: &mut *source,
            indices: &indices,
            snapshot: &mut snapshot,
            input_rows,
        },
    )
    .await;

    match result {
        Ok(summary) => {
            if let Some(snapshot) = snapshot.as_mut() {
                if snapshot.is_empty() {
                    snapshot.write(embeddings_dataframe(&[])?).await?;
                }
                snapshot.close().await?;
            }
            Ok(summary)
        }
        Err(error) => {
            if let Some(snapshot) = snapshot.as_mut()
                && let Err(abort_error) = snapshot.abort().await
            {
                context.callbacks.warning(
                    GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                    &format!(
                        "embedding {} snapshot abort failed after primary error: {abort_error}",
                        field.name
                    ),
                );
            }
            Err(error)
        }
    }
}

async fn process_field_inner(
    config: &GraphRagConfig,
    context: &mut PipelineRunContext,
    field: &EmbeddingField,
    schema: &VectorIndexSchema,
    dependencies: &FieldDependencies,
    input: FieldInput<'_>,
) -> Result<FieldSummary> {
    let flush_size = config
        .embed_text
        .batch_size
        .saturating_mul(config.concurrent_requests.max(1))
        .max(1);
    let operation_config = EmbeddingOperationConfig {
        batch_size: config.embed_text.batch_size,
        batch_max_tokens: config.embed_text.batch_max_tokens,
        concurrency: config.concurrent_requests.max(1),
        chunk_overlap: CHUNK_OVERLAP,
        expected_vector_size: schema.vector_size,
        model_instance_name: config.embed_text.model_instance_name.clone(),
        embedding_name: field.name.to_owned(),
    };

    let mut rows = input.source.rows();
    let mut buffer = Vec::with_capacity(flush_size);
    let mut summary = FieldSummary {
        name: field.name.to_owned(),
        source_table: field.source_table.to_owned(),
        input_rows: input.input_rows,
        embedded_rows: 0,
        skipped_rows: 0,
        snippet_count: 0,
        request_count: 0,
        index_name: schema.index_name.clone(),
    };
    let mut completed = 0usize;
    let mut seen_source_ids = BTreeSet::new();

    while let Some(row) = rows.next().await {
        let source_row = source_row(field, input.indices, &row?)?;
        if !seen_source_ids.insert(source_row.id.clone()) {
            return Err(invalid_data(
                GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                &format!("{} duplicate source id {}", field.name, source_row.id),
            ));
        }
        buffer.push(source_row);
        if buffer.len() >= flush_size {
            flush_buffer(
                context,
                &operation_config,
                dependencies,
                schema,
                FlushState {
                    buffer: &mut buffer,
                    snapshot: input.snapshot,
                    summary: &mut summary,
                    completed: &mut completed,
                    total: input.input_rows,
                },
            )
            .await?;
        }
    }
    if !buffer.is_empty() {
        flush_buffer(
            context,
            &operation_config,
            dependencies,
            schema,
            FlushState {
                buffer: &mut buffer,
                snapshot: input.snapshot,
                summary: &mut summary,
                completed: &mut completed,
                total: input.input_rows,
            },
        )
        .await?;
    }
    context.callbacks.progress(
        GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
        completed,
        Some(input.input_rows),
    );
    Ok(summary)
}

async fn flush_buffer(
    context: &mut PipelineRunContext,
    operation_config: &EmbeddingOperationConfig,
    dependencies: &FieldDependencies,
    schema: &VectorIndexSchema,
    state: FlushState<'_>,
) -> Result<()> {
    let rows = std::mem::take(state.buffer);
    let output = embed_text_rows(
        &rows,
        operation_config,
        Arc::clone(&dependencies.model),
        Arc::clone(&dependencies.tokenizer),
        dependencies.embedding_cache.as_ref().map(Arc::clone),
    )
    .await?;
    dependencies
        .vector_store
        .upsert_documents(schema, &output.documents)
        .await?;
    if let Some(snapshot) = state.snapshot.as_mut()
        && !output.documents.is_empty()
    {
        snapshot
            .write(embeddings_dataframe(&output.documents)?)
            .await?;
    }
    state.summary.embedded_rows = state
        .summary
        .embedded_rows
        .saturating_add(output.documents.len());
    state.summary.skipped_rows = state
        .summary
        .skipped_rows
        .saturating_add(output.skipped_rows);
    state.summary.snippet_count = state
        .summary
        .snippet_count
        .saturating_add(output.snippet_count);
    state.summary.request_count = state
        .summary
        .request_count
        .saturating_add(output.request_count);
    context.stats.llm_request_count = context
        .stats
        .llm_request_count
        .saturating_add(output.request_count);
    context.stats.cache_hit_count = context
        .stats
        .cache_hit_count
        .saturating_add(output.cache_hits);
    context.stats.cache_miss_count = context
        .stats
        .cache_miss_count
        .saturating_add(output.cache_misses);
    context.stats.input_token_count = context
        .stats
        .input_token_count
        .saturating_add(output.input_tokens);
    *state.completed = state.completed.saturating_add(output.attempted_rows);
    context.callbacks.progress(
        GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
        *state.completed,
        Some(state.total),
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct SourceIndices {
    id: usize,
    text_columns: BTreeMap<&'static str, usize>,
}

impl SourceIndices {
    fn new(field: &EmbeddingField, columns: &[String]) -> Result<Self> {
        let id = find_column(columns, field.id_column)?;
        let mut text_columns = BTreeMap::new();
        for column in field.text_columns {
            text_columns.insert(*column, find_column(columns, column)?);
        }
        Ok(Self { id, text_columns })
    }
}

fn source_row(
    field: &EmbeddingField,
    indices: &SourceIndices,
    row: &Row<'static>,
) -> Result<EmbeddingSourceRow> {
    let id = required_string(row, indices.id, field.name, field.id_column)?;
    let text = match field.mapper {
        TextMapper::Single(column) => {
            nullable_string(row, indices.text_columns[column], field.name, column)?
        }
        TextMapper::EntityDescription => {
            let title = nullable_string(row, indices.text_columns["title"], field.name, "title")?;
            let description = nullable_string(
                row,
                indices.text_columns["description"],
                field.name,
                "description",
            )?;
            let combined = format!("{title}:{description}");
            if combined == ":" {
                String::new()
            } else {
                combined
            }
        }
    };
    Ok(EmbeddingSourceRow { id, text })
}

fn required_string(
    row: &Row<'static>,
    index: usize,
    embedding_name: &str,
    column: &str,
) -> Result<String> {
    let value = row.0.get(index).ok_or_else(|| {
        invalid_data(
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
            &format!("{embedding_name} column {column} is missing"),
        )
    })?;
    let id = match value {
        AnyValue::String(value) => (*value).to_owned(),
        AnyValue::StringOwned(value) => value.to_string(),
        AnyValue::Null => {
            return Err(invalid_data(
                GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                &format!("{embedding_name} column {column} expected non-empty String, got Null"),
            ));
        }
        other => {
            return Err(invalid_data(
                GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                &format!(
                    "{embedding_name} column {column} expected non-empty String, got {other:?}"
                ),
            ));
        }
    };
    if id.is_empty() {
        return Err(invalid_data(
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
            &format!("{embedding_name} column {column} must not be empty"),
        ));
    }
    Ok(id)
}

fn nullable_string(
    row: &Row<'static>,
    index: usize,
    embedding_name: &str,
    column: &str,
) -> Result<String> {
    let value = row.0.get(index).ok_or_else(|| {
        invalid_data(
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
            &format!("{embedding_name} column {column} is missing"),
        )
    })?;
    match value {
        AnyValue::String(value) => Ok((*value).to_owned()),
        AnyValue::StringOwned(value) => Ok(value.to_string()),
        AnyValue::Null => Ok(String::new()),
        other => Err(invalid_data(
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
            &format!("{embedding_name} column {column} expected String or Null, got {other:?}"),
        )),
    }
}

fn find_column(columns: &[String], column: &str) -> Result<usize> {
    columns
        .iter()
        .position(|name| name == column)
        .ok_or_else(|| {
            invalid_data(
                GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                &format!("missing column {column}"),
            )
        })
}

fn embeddings_dataframe(documents: &[VectorDocument]) -> Result<DataFrame> {
    let ids = documents
        .iter()
        .map(|document| document.id.as_str())
        .collect::<Vec<_>>();
    let embedding_column: Column = if documents.is_empty() {
        Series::new_empty(
            "embedding".into(),
            &DataType::List(Box::new(DataType::Float32)),
        )
        .into()
    } else {
        let rows = documents
            .iter()
            .map(|document| Series::new("item".into(), document.vector.as_slice()))
            .collect::<Vec<_>>();
        Series::new("embedding".into(), rows).into()
    };
    DataFrame::new(
        documents.len(),
        vec![Series::new("id".into(), ids).into(), embedding_column],
    )
    .map_err(GraphLoomError::from)
}

fn embedding_fields() -> BTreeMap<&'static str, EmbeddingField> {
    BTreeMap::from([
        (
            TEXT_UNIT_TEXT_EMBEDDING,
            EmbeddingField {
                name: TEXT_UNIT_TEXT_EMBEDDING,
                source_table: "text_units",
                id_column: "id",
                text_columns: &["text"],
                mapper: TextMapper::Single("text"),
            },
        ),
        (
            ENTITY_DESCRIPTION_EMBEDDING,
            EmbeddingField {
                name: ENTITY_DESCRIPTION_EMBEDDING,
                source_table: "entities",
                id_column: "id",
                text_columns: &["title", "description"],
                mapper: TextMapper::EntityDescription,
            },
        ),
        (
            COMMUNITY_FULL_CONTENT_EMBEDDING,
            EmbeddingField {
                name: COMMUNITY_FULL_CONTENT_EMBEDDING,
                source_table: "community_reports",
                id_column: "id",
                text_columns: &["full_content"],
                mapper: TextMapper::Single("full_content"),
            },
        ),
    ])
}

#[cfg(test)]
mod tests {
    use polars_core::prelude::AnyValue;

    use super::*;

    fn row(values: Vec<AnyValue<'static>>) -> Row<'static> {
        Row(values)
    }

    #[test]
    fn test_should_fail_fast_on_non_string_text_columns() {
        let fields = embedding_fields();
        let text = fields.get(TEXT_UNIT_TEXT_EMBEDDING).expect("text field");
        let text_indices = SourceIndices {
            id: 0,
            text_columns: BTreeMap::from([("text", 1)]),
        };
        let error = source_row(
            text,
            &text_indices,
            &row(vec![AnyValue::String("tu-1"), AnyValue::Int64(7)]),
        )
        .expect_err("int text should fail");
        assert!(error.to_string().contains("text_unit_text"));
        assert!(error.to_string().contains("text"));

        let entity = fields
            .get(ENTITY_DESCRIPTION_EMBEDDING)
            .expect("entity field");
        let entity_indices = SourceIndices {
            id: 0,
            text_columns: BTreeMap::from([("title", 1), ("description", 2)]),
        };
        let error = source_row(
            entity,
            &entity_indices,
            &row(vec![
                AnyValue::String("e-1"),
                AnyValue::Boolean(true),
                AnyValue::String("Engineer"),
            ]),
        )
        .expect_err("boolean title should fail");
        assert!(error.to_string().contains("title"));

        let error = source_row(
            entity,
            &entity_indices,
            &row(vec![
                AnyValue::String("e-1"),
                AnyValue::String("Alice"),
                AnyValue::Float64(1.0),
            ]),
        )
        .expect_err("float description should fail");
        assert!(error.to_string().contains("description"));

        let community = fields
            .get(COMMUNITY_FULL_CONTENT_EMBEDDING)
            .expect("community field");
        let community_indices = SourceIndices {
            id: 0,
            text_columns: BTreeMap::from([("full_content", 1)]),
        };
        let error = source_row(
            community,
            &community_indices,
            &row(vec![AnyValue::String("report-1"), AnyValue::Int64(42)]),
        )
        .expect_err("int full_content should fail");
        assert!(error.to_string().contains("community_full_content"));
        assert!(error.to_string().contains("full_content"));
        assert!(error.to_string().contains("Int64"));
    }

    #[test]
    fn test_should_fail_fast_on_non_string_id_and_skip_null_text() {
        let fields = embedding_fields();
        let text = fields.get(TEXT_UNIT_TEXT_EMBEDDING).expect("text field");
        let indices = SourceIndices {
            id: 0,
            text_columns: BTreeMap::from([("text", 1)]),
        };

        let error = source_row(
            text,
            &indices,
            &row(vec![AnyValue::Int64(1), AnyValue::Null]),
        )
        .expect_err("numeric id should fail");
        assert!(error.to_string().contains("expected non-empty String"));

        let source = source_row(
            text,
            &indices,
            &row(vec![AnyValue::String("tu-1"), AnyValue::Null]),
        )
        .expect("null text should map to empty text");
        assert_eq!(source.text, "");
    }

    #[test]
    fn test_should_preserve_entity_title_description_separator() {
        let fields = embedding_fields();
        let entity = fields
            .get(ENTITY_DESCRIPTION_EMBEDDING)
            .expect("entity field");
        let indices = SourceIndices {
            id: 0,
            text_columns: BTreeMap::from([("title", 1), ("description", 2)]),
        };

        let title_only = source_row(
            entity,
            &indices,
            &row(vec![
                AnyValue::String("e-1"),
                AnyValue::String("Alice"),
                AnyValue::Null,
            ]),
        )
        .expect("title only");
        assert_eq!(title_only.text, "Alice:");

        let description_only = source_row(
            entity,
            &indices,
            &row(vec![
                AnyValue::String("e-2"),
                AnyValue::Null,
                AnyValue::String("Engineer"),
            ]),
        )
        .expect("description only");
        assert_eq!(description_only.text, ":Engineer");

        let empty = source_row(
            entity,
            &indices,
            &row(vec![
                AnyValue::String("e-3"),
                AnyValue::Null,
                AnyValue::Null,
            ]),
        )
        .expect("empty entity text");
        assert_eq!(empty.text, "");
    }
}
