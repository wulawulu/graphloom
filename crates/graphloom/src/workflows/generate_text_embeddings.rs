//! Text embedding generation workflow.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use futures_util::StreamExt;
use graphloom_llm::TiktokenTokenizer;
use graphloom_storage::{Table, TableProvider};
use graphloom_vectors::{VectorDocument, VectorIndexSchema, VectorStore, create_vector_store};
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

        let model_config = config
            .embedding_models
            .get(&config.embed_text.embedding_model_id)
            .ok_or_else(|| GraphLoomError::InvalidData {
                workflow: GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
                message: format!(
                    "embedding model {} is not configured",
                    config.embed_text.embedding_model_id
                ),
            })?;
        let model = resolve_embedding_model(
            config,
            context,
            &config.embed_text.embedding_model_id,
            &config.embed_text.model_instance_name,
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
        )?;
        let encoding_model =
            resolve_embedding_encoding_model(config, &config.embed_text.embedding_model_id);
        let tokenizer: Arc<dyn graphloom_llm::Tokenizer> =
            Arc::new(TiktokenTokenizer::new(encoding_model)?);
        let vector_store = match &context.vector_store {
            Some(store) => Arc::clone(store),
            None => create_vector_store(&config.vector_store).await?,
        };

        let field_map = embedding_fields();
        let snapshot_provider = if config.snapshots.embeddings {
            Some(context.output_table_provider.child(Some("embeddings"))?)
        } else {
            None
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
                .output_table_provider
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
            vector_store.ensure_index(&schema).await?;
            let summary = process_field(
                config,
                context,
                field,
                &schema,
                Arc::clone(&vector_store),
                Arc::clone(&model),
                model_config,
                Arc::clone(&tokenizer),
                snapshot_provider.as_ref().map(Arc::clone),
            )
            .await?;
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

#[allow(
    clippy::too_many_arguments,
    reason = "workflow field execution passes explicit dependencies without hiding GraphRagConfig \
              or PipelineRunContext in operations"
)]
async fn process_field(
    config: &GraphRagConfig,
    context: &mut PipelineRunContext,
    field: &EmbeddingField,
    schema: &VectorIndexSchema,
    vector_store: Arc<dyn VectorStore>,
    model: Arc<dyn graphloom_llm::EmbeddingModel>,
    model_config: &graphloom_llm::ModelConfig,
    tokenizer: Arc<dyn graphloom_llm::Tokenizer>,
    snapshot_provider: Option<Arc<dyn TableProvider>>,
) -> Result<FieldSummary> {
    let mut source = context
        .output_table_provider
        .open(field.source_table, false)
        .await?;
    let input_rows = source.len();
    let columns = source.column_names();
    let indices = SourceIndices::new(field, &columns)?;
    let mut snapshot = match snapshot_provider {
        Some(provider) => Some(provider.open(field.name, true).await?),
        None => None,
    };

    let result = process_field_inner(
        config,
        context,
        field,
        schema,
        vector_store,
        model,
        model_config,
        tokenizer,
        &mut *source,
        &indices,
        &mut snapshot,
        input_rows,
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
            if let Some(snapshot) = snapshot.as_mut() {
                snapshot.abort().await?;
            }
            Err(error)
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "streaming field loop keeps source, snapshot, model, tokenizer, and store \
              dependencies explicit"
)]
async fn process_field_inner(
    config: &GraphRagConfig,
    context: &mut PipelineRunContext,
    field: &EmbeddingField,
    schema: &VectorIndexSchema,
    vector_store: Arc<dyn VectorStore>,
    model: Arc<dyn graphloom_llm::EmbeddingModel>,
    model_config: &graphloom_llm::ModelConfig,
    tokenizer: Arc<dyn graphloom_llm::Tokenizer>,
    source: &mut dyn Table,
    indices: &SourceIndices,
    snapshot: &mut Option<Box<dyn Table>>,
    input_rows: usize,
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

    let mut rows = source.rows();
    let mut buffer = Vec::with_capacity(flush_size);
    let mut summary = FieldSummary {
        name: field.name.to_owned(),
        source_table: field.source_table.to_owned(),
        input_rows,
        embedded_rows: 0,
        skipped_rows: 0,
        snippet_count: 0,
        request_count: 0,
        index_name: schema.index_name.clone(),
    };
    let mut completed = 0usize;

    while let Some(row) = rows.next().await {
        buffer.push(source_row(field, indices, &row?)?);
        if buffer.len() >= flush_size {
            flush_buffer(
                context,
                &mut buffer,
                &operation_config,
                &vector_store,
                schema,
                Arc::clone(&model),
                model_config,
                Arc::clone(&tokenizer),
                snapshot,
                &mut summary,
                &mut completed,
                input_rows,
            )
            .await?;
        }
    }
    if !buffer.is_empty() {
        flush_buffer(
            context,
            &mut buffer,
            &operation_config,
            &vector_store,
            schema,
            model,
            model_config,
            tokenizer,
            snapshot,
            &mut summary,
            &mut completed,
            input_rows,
        )
        .await?;
    }
    context.callbacks.progress(
        GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
        completed,
        Some(input_rows),
    );
    Ok(summary)
}

#[allow(
    clippy::too_many_arguments,
    reason = "flush boundary updates operation output, vector store, snapshots, progress, and \
              shared stats atomically"
)]
async fn flush_buffer(
    context: &mut PipelineRunContext,
    buffer: &mut Vec<EmbeddingSourceRow>,
    operation_config: &EmbeddingOperationConfig,
    vector_store: &Arc<dyn VectorStore>,
    schema: &VectorIndexSchema,
    model: Arc<dyn graphloom_llm::EmbeddingModel>,
    model_config: &graphloom_llm::ModelConfig,
    tokenizer: Arc<dyn graphloom_llm::Tokenizer>,
    snapshot: &mut Option<Box<dyn Table>>,
    summary: &mut FieldSummary,
    completed: &mut usize,
    total: usize,
) -> Result<()> {
    let rows = std::mem::take(buffer);
    let output = embed_text_rows(
        &rows,
        operation_config,
        model,
        model_config,
        tokenizer,
        context.cache.as_ref().map(Arc::clone),
    )
    .await?;
    vector_store
        .upsert_documents(schema, &output.documents)
        .await?;
    if let Some(snapshot) = snapshot.as_mut()
        && !output.documents.is_empty()
    {
        snapshot
            .write(embeddings_dataframe(&output.documents)?)
            .await?;
    }
    summary.embedded_rows = summary.embedded_rows.saturating_add(output.documents.len());
    summary.skipped_rows = summary.skipped_rows.saturating_add(output.skipped_rows);
    summary.snippet_count = summary.snippet_count.saturating_add(output.snippet_count);
    summary.request_count = summary.request_count.saturating_add(output.request_count);
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
    *completed = completed.saturating_add(output.attempted_rows);
    context
        .callbacks
        .progress(GENERATE_TEXT_EMBEDDINGS_WORKFLOW, *completed, Some(total));
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
    let id = required_string(row, indices.id, field.id_column)?;
    if id.is_empty() {
        return Err(invalid_data(
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
            &format!("{} source id must not be empty", field.name),
        ));
    }
    let text = match field.mapper {
        TextMapper::Single(column) => optional_string(row, indices.text_columns[column]),
        TextMapper::EntityDescription => {
            let title = optional_string(row, indices.text_columns["title"]);
            let description = optional_string(row, indices.text_columns["description"]);
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

fn required_string(row: &Row<'static>, index: usize, column: &str) -> Result<String> {
    optional_string_value(row.0.get(index)).ok_or_else(|| {
        invalid_data(
            GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
            &format!("missing string column {column}"),
        )
    })
}

fn optional_string(row: &Row<'static>, index: usize) -> String {
    optional_string_value(row.0.get(index)).unwrap_or_default()
}

fn optional_string_value(value: Option<&AnyValue<'_>>) -> Option<String> {
    match value {
        Some(AnyValue::String(value)) => Some((*value).to_owned()),
        Some(AnyValue::StringOwned(value)) => Some(value.to_string()),
        Some(AnyValue::Null) => Some(String::new()),
        _ => None,
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
