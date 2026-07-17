//! `LanceDB` vector store provider.

use std::{collections::BTreeSet, fmt, sync::Arc};

use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, Float64Array, Int64Array, RecordBatch,
    RecordBatchIterator, RecordBatchReader, StringArray, types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use chrono::{Datelike, Timelike, Utc};
use futures_util::TryStreamExt;
use lancedb::{
    Connection, Table, connect,
    expr::{col, is_in, lit},
    index::{Index, IndexType, vector::IvfFlatIndexBuilder},
    query::{ExecutableQuery, QueryBase, Select},
};

use crate::{
    Result, VectorDocument, VectorError, VectorIndexSchema, VectorSearchResult, VectorStore,
    VectorStoreConfig,
};

const DUMMY_ID: &str = "__DUMMY__";
const CREATE_DATE_FIELD: &str = "create_date";
const UPDATE_DATE_FIELD: &str = "update_date";

/// LanceDB-backed vector store.
pub struct LanceDbVectorStore {
    connection: Connection,
}

impl fmt::Debug for LanceDbVectorStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LanceDbVectorStore")
            .finish_non_exhaustive()
    }
}

impl LanceDbVectorStore {
    /// Connect to a `LanceDB` database.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration is invalid or `LanceDB` cannot connect.
    pub async fn connect(config: &VectorStoreConfig) -> Result<Self> {
        config.validate()?;
        let connection = connect(&config.db_uri).execute().await?;
        Ok(Self { connection })
    }

    async fn table_exists(&self, name: &str) -> Result<bool> {
        Ok(self
            .connection
            .table_names()
            .execute()
            .await?
            .iter()
            .any(|table_name| table_name == name))
    }

    async fn open_table(&self, schema: &VectorIndexSchema) -> Result<Table> {
        self.connection
            .open_table(&schema.index_name)
            .execute()
            .await
            .map_err(VectorError::from)
    }

    async fn create_indexed_table(&self, schema: &VectorIndexSchema) -> Result<Table> {
        let dummy = VectorDocument {
            id: DUMMY_ID.to_owned(),
            vector: vec![0.0; schema.vector_size],
        };
        let table = self
            .connection
            .create_table(&schema.index_name, documents_reader(schema, &[dummy])?)
            .execute()
            .await?;
        create_vector_index(&table, schema).await?;
        delete_dummy(&table, schema).await?;
        Ok(table)
    }
}

#[async_trait]
impl VectorStore for LanceDbVectorStore {
    async fn ensure_index(&self, schema: &VectorIndexSchema) -> Result<()> {
        schema.validate()?;
        if self.table_exists(&schema.index_name).await? {
            let table = self.open_table(schema).await?;
            validate_table_schema(schema, table.schema().await?.as_ref())?;
            ensure_vector_index(&table, schema).await?;
            return Ok(());
        }

        self.create_indexed_table(schema).await?;
        Ok(())
    }

    async fn reset_index(&self, schema: &VectorIndexSchema) -> Result<()> {
        schema.validate()?;
        if self.table_exists(&schema.index_name).await? {
            self.connection.drop_table(&schema.index_name, &[]).await?;
        }
        self.create_indexed_table(schema).await?;
        Ok(())
    }

    async fn upsert_documents(
        &self,
        schema: &VectorIndexSchema,
        documents: &[VectorDocument],
    ) -> Result<()> {
        schema.validate()?;
        if documents.is_empty() {
            return Ok(());
        }
        validate_documents(schema, documents)?;
        self.ensure_index(schema).await?;
        let table = self.open_table(schema).await?;
        let reader = documents_reader(schema, documents)?;

        if contains_existing_id(&table, schema, documents).await? {
            let mut upsert = table.merge_insert(&[schema.id_field.as_str()]);
            upsert
                .when_matched_update_all(None)
                .when_not_matched_insert_all();
            upsert.execute(reader).await?;
            ensure_vector_index(&table, schema).await?;
        } else {
            table.add(reader).execute().await?;
            ensure_vector_index(&table, schema).await?;
        }
        Ok(())
    }

    async fn count(&self, schema: &VectorIndexSchema) -> Result<usize> {
        schema.validate()?;
        if !self.table_exists(&schema.index_name).await? {
            return Err(VectorError::MissingIndex {
                index_name: schema.index_name.clone(),
            });
        }
        let table = self.open_table(schema).await?;
        validate_table_schema(schema, table.schema().await?.as_ref())?;
        table.count_rows(None).await.map_err(VectorError::from)
    }

    async fn ids(&self, schema: &VectorIndexSchema) -> Result<Vec<String>> {
        schema.validate()?;
        let table = self.open_table(schema).await?;
        let mut stream = table
            .query()
            .select(Select::columns(&[&schema.id_field]))
            .execute()
            .await?;
        let mut ids = Vec::new();

        while let Some(batch) = stream.try_next().await? {
            let id_column = batch
                .column_by_name(&schema.id_field)
                .and_then(|column| column.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| VectorError::InvalidDocument {
                    index_name: schema.index_name.clone(),
                    message: format!("id field {} is not Utf8", schema.id_field),
                })?;
            for row_index in 0..batch.num_rows() {
                if !id_column.is_null(row_index) {
                    ids.push(id_column.value(row_index).to_owned());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    async fn get_by_id(
        &self,
        schema: &VectorIndexSchema,
        id: &str,
    ) -> Result<Option<VectorDocument>> {
        schema.validate()?;
        if !self.table_exists(&schema.index_name).await? {
            return Err(VectorError::MissingIndex {
                index_name: schema.index_name.clone(),
            });
        }
        let table = self.open_table(schema).await?;
        validate_table_schema(schema, table.schema().await?.as_ref())?;
        let mut stream = table
            .query()
            .select(Select::columns(&[&schema.id_field, &schema.vector_field]))
            .execute()
            .await?;

        while let Some(batch) = stream.try_next().await? {
            let ids = batch
                .column_by_name(&schema.id_field)
                .and_then(|column| column.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| VectorError::InvalidDocument {
                    index_name: schema.index_name.clone(),
                    message: format!("id field {} is not Utf8", schema.id_field),
                })?;
            let vectors = batch
                .column_by_name(&schema.vector_field)
                .and_then(|column| column.as_any().downcast_ref::<FixedSizeListArray>())
                .ok_or_else(|| VectorError::InvalidDocument {
                    index_name: schema.index_name.clone(),
                    message: format!("vector field {} is not FixedSizeList", schema.vector_field),
                })?;
            for row_index in 0..batch.num_rows() {
                if !ids.is_null(row_index) && ids.value(row_index) == id {
                    return Ok(Some(VectorDocument {
                        id: id.to_owned(),
                        vector: vector_value(schema, vectors, row_index)?,
                    }));
                }
            }
        }
        Ok(None)
    }

    async fn similarity_search_by_vector(
        &self,
        schema: &VectorIndexSchema,
        query_vector: &[f32],
        k: usize,
        include_vectors: bool,
    ) -> Result<Vec<VectorSearchResult>> {
        validate_search_query(schema, query_vector, k)?;
        if !self.table_exists(&schema.index_name).await? {
            return Err(VectorError::MissingIndex {
                index_name: schema.index_name.clone(),
            });
        }
        let table = self.open_table(schema).await?;
        validate_table_schema(schema, table.schema().await?.as_ref())?;
        let columns = if include_vectors {
            vec![schema.id_field.as_str(), schema.vector_field.as_str()]
        } else {
            vec![schema.id_field.as_str()]
        };
        let mut stream = table
            .query()
            .nearest_to(query_vector)?
            .column(&schema.vector_field)
            .limit(k)
            .select(Select::columns(&columns))
            .execute()
            .await?;
        let mut results = Vec::with_capacity(k);
        while let Some(batch) = stream.try_next().await? {
            append_search_batch(schema, &batch, include_vectors, &mut results)?;
        }
        Ok(results)
    }
}

fn validate_search_query(schema: &VectorIndexSchema, query_vector: &[f32], k: usize) -> Result<()> {
    schema.validate()?;
    if k == 0 {
        return Err(VectorError::InvalidQuery {
            index_name: schema.index_name.clone(),
            message: "k must be greater than zero".to_owned(),
        });
    }
    if query_vector.is_empty() {
        return Err(VectorError::InvalidQuery {
            index_name: schema.index_name.clone(),
            message: "query vector must not be empty".to_owned(),
        });
    }
    if query_vector.len() != schema.vector_size {
        return Err(VectorError::InvalidQuery {
            index_name: schema.index_name.clone(),
            message: format!(
                "query vector dimension mismatch: expected {}, got {}",
                schema.vector_size,
                query_vector.len()
            ),
        });
    }
    if query_vector.iter().any(|value| !value.is_finite()) {
        return Err(VectorError::InvalidQuery {
            index_name: schema.index_name.clone(),
            message: "query vector contains a non-finite value".to_owned(),
        });
    }
    Ok(())
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Float64 ANN distances are range- and finiteness-checked before conversion"
)]
fn append_search_batch(
    schema: &VectorIndexSchema,
    batch: &RecordBatch,
    include_vectors: bool,
    results: &mut Vec<VectorSearchResult>,
) -> Result<()> {
    let ids = batch
        .column_by_name(&schema.id_field)
        .and_then(|column| column.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| invalid_search_result(schema, "id column is not Utf8"))?;
    let distance_column = batch
        .column_by_name("_distance")
        .ok_or_else(|| invalid_search_result(schema, "_distance column is missing"))?;
    let distances_f32 = distance_column.as_any().downcast_ref::<Float32Array>();
    let distances_f64 = distance_column.as_any().downcast_ref::<Float64Array>();
    if distances_f32.is_none() && distances_f64.is_none() {
        return Err(invalid_search_result(
            schema,
            "_distance column is neither Float32 nor Float64",
        ));
    }
    let vectors = if include_vectors {
        Some(
            batch
                .column_by_name(&schema.vector_field)
                .and_then(|column| column.as_any().downcast_ref::<FixedSizeListArray>())
                .ok_or_else(|| {
                    invalid_search_result(schema, "vector column is not FixedSizeList")
                })?,
        )
    } else {
        None
    };
    for row_index in 0..batch.num_rows() {
        if ids.is_null(row_index) || ids.value(row_index).is_empty() {
            return Err(invalid_search_result(schema, "search result id is empty"));
        }
        if distance_column.is_null(row_index) {
            return Err(invalid_search_result(
                schema,
                "search result distance is null",
            ));
        }
        let distance = if let Some(distances) = distances_f32 {
            distances.value(row_index)
        } else if let Some(distances) = distances_f64 {
            let value = distances.value(row_index);
            if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
                return Err(invalid_search_result(
                    schema,
                    "search result distance cannot be represented as finite Float32",
                ));
            }
            value as f32
        } else {
            return Err(invalid_search_result(
                schema,
                "search result distance is unavailable",
            ));
        };
        if !distance.is_finite() {
            return Err(invalid_search_result(
                schema,
                "search result distance is non-finite",
            ));
        }
        let vector = vectors.map_or_else(
            || Ok(Vec::new()),
            |vectors| vector_value(schema, vectors, row_index),
        )?;
        results.push(VectorSearchResult {
            document: VectorDocument {
                id: ids.value(row_index).to_owned(),
                vector,
            },
            score: 1.0 - distance.abs(),
        });
    }
    Ok(())
}

fn invalid_search_result(schema: &VectorIndexSchema, message: &str) -> VectorError {
    VectorError::InvalidQuery {
        index_name: schema.index_name.clone(),
        message: message.to_owned(),
    }
}

fn arrow_schema(schema: &VectorIndexSchema) -> Result<SchemaRef> {
    let vector_size =
        i32::try_from(schema.vector_size).map_err(|source| VectorError::InvalidConfig {
            message: format!(
                "vector_size {} does not fit i32: {source}",
                schema.vector_size
            ),
        })?;
    let mut fields = vec![
        Field::new(&schema.id_field, DataType::Utf8, true),
        Field::new(
            &schema.vector_field,
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                vector_size,
            ),
            true,
        ),
        Field::new(CREATE_DATE_FIELD, DataType::Utf8, true),
        Field::new(UPDATE_DATE_FIELD, DataType::Utf8, true),
    ];
    fields.extend(timestamp_component_fields(CREATE_DATE_FIELD));
    fields.extend(timestamp_component_fields(UPDATE_DATE_FIELD));
    Ok(Arc::new(Schema::new(fields)))
}

fn timestamp_component_fields(prefix: &str) -> Vec<Field> {
    vec![
        Field::new(format!("{prefix}_year"), DataType::Int64, true),
        Field::new(format!("{prefix}_month"), DataType::Int64, true),
        Field::new(format!("{prefix}_month_name"), DataType::Utf8, true),
        Field::new(format!("{prefix}_day"), DataType::Int64, true),
        Field::new(format!("{prefix}_day_of_week"), DataType::Utf8, true),
        Field::new(format!("{prefix}_hour"), DataType::Int64, true),
        Field::new(format!("{prefix}_quarter"), DataType::Int64, true),
    ]
}

async fn create_vector_index(table: &Table, schema: &VectorIndexSchema) -> Result<()> {
    table
        .create_index(
            &[schema.vector_field.as_str()],
            Index::IvfFlat(IvfFlatIndexBuilder::default()),
        )
        .execute()
        .await?;
    Ok(())
}

async fn delete_dummy(table: &Table, schema: &VectorIndexSchema) -> Result<()> {
    let predicate = format!("{} = '{DUMMY_ID}'", schema.id_field);
    table.delete(&predicate).await?;
    Ok(())
}

async fn ensure_vector_index(table: &Table, schema: &VectorIndexSchema) -> Result<()> {
    let indices = table.list_indices().await?;
    if indices.iter().any(|index| {
        index.index_type == IndexType::IvfFlat
            && index.columns.as_slice() == [schema.vector_field.as_str()]
    }) {
        return Ok(());
    }

    if table.count_rows(None).await? == 0 {
        let dummy = VectorDocument {
            id: DUMMY_ID.to_owned(),
            vector: vec![0.0; schema.vector_size],
        };
        table
            .add(documents_reader(schema, &[dummy])?)
            .execute()
            .await?;
        create_vector_index(table, schema).await?;
        delete_dummy(table, schema).await?;
    } else {
        create_vector_index(table, schema).await?;
    }
    Ok(())
}

async fn contains_existing_id(
    table: &Table,
    schema: &VectorIndexSchema,
    documents: &[VectorDocument],
) -> Result<bool> {
    let filter = is_in(
        col(&schema.id_field),
        documents
            .iter()
            .map(|document| lit(document.id.clone()))
            .collect(),
    );
    let mut rows = table
        .query()
        .select(Select::columns(&[&schema.id_field]))
        .only_if_expr(filter)
        .limit(1)
        .execute()
        .await?;
    while let Some(batch) = rows.try_next().await? {
        if batch.num_rows() > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_table_schema(schema: &VectorIndexSchema, table_schema: &Schema) -> Result<()> {
    let id_field = table_schema
        .field_with_name(&schema.id_field)
        .map_err(|source| VectorError::InvalidConfig {
            message: format!(
                "index {} missing id field {}: {source}",
                schema.index_name, schema.id_field
            ),
        })?;
    if id_field.data_type() != &DataType::Utf8 {
        return Err(VectorError::InvalidConfig {
            message: format!(
                "index {} id field {} must be Utf8, got {:?}",
                schema.index_name,
                schema.id_field,
                id_field.data_type(),
            ),
        });
    }

    let vector_field = table_schema
        .field_with_name(&schema.vector_field)
        .map_err(|source| VectorError::InvalidConfig {
            message: format!(
                "index {} missing vector field {}: {source}",
                schema.index_name, schema.vector_field
            ),
        })?;
    let DataType::FixedSizeList(item, size) = vector_field.data_type() else {
        return Err(VectorError::InvalidConfig {
            message: format!(
                "index {} vector field {} must be FixedSizeList(Float32, {}), got {:?}",
                schema.index_name,
                schema.vector_field,
                schema.vector_size,
                vector_field.data_type(),
            ),
        });
    };
    if item.data_type() != &DataType::Float32
        || usize::try_from(*size).ok() != Some(schema.vector_size)
    {
        return Err(VectorError::InvalidConfig {
            message: format!(
                "index {} vector field {} must have Float32 size {}, got {:?}",
                schema.index_name,
                schema.vector_field,
                schema.vector_size,
                vector_field.data_type(),
            ),
        });
    }
    for expected in [
        Field::new(CREATE_DATE_FIELD, DataType::Utf8, true),
        Field::new(UPDATE_DATE_FIELD, DataType::Utf8, true),
    ]
    .into_iter()
    .chain(timestamp_component_fields(CREATE_DATE_FIELD))
    .chain(timestamp_component_fields(UPDATE_DATE_FIELD))
    {
        let actual = table_schema
            .field_with_name(expected.name())
            .map_err(|source| VectorError::InvalidConfig {
                message: format!(
                    "index {} missing GraphRAG metadata field {}: {source}",
                    schema.index_name,
                    expected.name(),
                ),
            })?;
        if actual.data_type() != expected.data_type() {
            return Err(VectorError::InvalidConfig {
                message: format!(
                    "index {} metadata field {} must be {:?}, got {:?}",
                    schema.index_name,
                    expected.name(),
                    expected.data_type(),
                    actual.data_type(),
                ),
            });
        }
    }
    Ok(())
}

fn validate_documents(schema: &VectorIndexSchema, documents: &[VectorDocument]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for document in documents {
        if document.id.is_empty() {
            return Err(VectorError::InvalidDocument {
                index_name: schema.index_name.clone(),
                message: "document id must not be empty".to_owned(),
            });
        }
        if !ids.insert(document.id.as_str()) {
            return Err(VectorError::InvalidDocument {
                index_name: schema.index_name.clone(),
                message: format!("duplicate document id {} in one upsert", document.id),
            });
        }
        if document.vector.len() != schema.vector_size {
            return Err(VectorError::InvalidDocument {
                index_name: schema.index_name.clone(),
                message: format!(
                    "document {} vector dimension expected {}, got {}",
                    document.id,
                    schema.vector_size,
                    document.vector.len(),
                ),
            });
        }
        if document.vector.iter().any(|value| !value.is_finite()) {
            return Err(VectorError::InvalidDocument {
                index_name: schema.index_name.clone(),
                message: format!("document {} vector contains non-finite value", document.id),
            });
        }
    }
    Ok(())
}

fn documents_reader(
    schema: &VectorIndexSchema,
    documents: &[VectorDocument],
) -> Result<Box<dyn RecordBatchReader + Send>> {
    let arrow_schema = arrow_schema(schema)?;
    let vector_size =
        i32::try_from(schema.vector_size).map_err(|source| VectorError::InvalidConfig {
            message: format!(
                "vector_size {} does not fit i32: {source}",
                schema.vector_size
            ),
        })?;
    let ids = StringArray::from_iter_values(documents.iter().map(|document| document.id.as_str()));
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        documents.iter().map(|document| {
            Some(
                document
                    .vector
                    .iter()
                    .copied()
                    .map(Some)
                    .collect::<Vec<_>>(),
            )
        }),
        vector_size,
    );
    let timestamps = documents
        .iter()
        .map(|_| TimestampMetadata::now())
        .collect::<Vec<_>>();
    let mut columns: Vec<ArrayRef> = vec![Arc::new(ids), Arc::new(vectors)];
    columns.extend(timestamp_arrays(&timestamps));
    let batch = RecordBatch::try_new(Arc::clone(&arrow_schema), columns)?;
    Ok(Box::new(RecordBatchIterator::new(
        vec![Ok(batch)],
        arrow_schema,
    )))
}

#[derive(Debug)]
struct TimestampMetadata {
    iso: String,
    year: i64,
    month: i64,
    month_name: String,
    day: i64,
    day_of_week: String,
    hour: i64,
    quarter: i64,
}

impl TimestampMetadata {
    fn now() -> Self {
        let now = Utc::now();
        let month = now.month();
        Self {
            iso: now.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string(),
            year: i64::from(now.year()),
            month: i64::from(month),
            month_name: now.format("%B").to_string(),
            day: i64::from(now.day()),
            day_of_week: now.format("%A").to_string(),
            hour: i64::from(now.hour()),
            quarter: i64::from((month - 1) / 3 + 1),
        }
    }
}

fn timestamp_arrays(timestamps: &[TimestampMetadata]) -> Vec<ArrayRef> {
    let row_count = timestamps.len();
    vec![
        Arc::new(StringArray::from_iter_values(
            timestamps.iter().map(|timestamp| timestamp.iso.as_str()),
        )),
        Arc::new(StringArray::new_null(row_count)),
        Arc::new(Int64Array::from_iter_values(
            timestamps.iter().map(|timestamp| timestamp.year),
        )),
        Arc::new(Int64Array::from_iter_values(
            timestamps.iter().map(|timestamp| timestamp.month),
        )),
        Arc::new(StringArray::from_iter_values(
            timestamps
                .iter()
                .map(|timestamp| timestamp.month_name.as_str()),
        )),
        Arc::new(Int64Array::from_iter_values(
            timestamps.iter().map(|timestamp| timestamp.day),
        )),
        Arc::new(StringArray::from_iter_values(
            timestamps
                .iter()
                .map(|timestamp| timestamp.day_of_week.as_str()),
        )),
        Arc::new(Int64Array::from_iter_values(
            timestamps.iter().map(|timestamp| timestamp.hour),
        )),
        Arc::new(Int64Array::from_iter_values(
            timestamps.iter().map(|timestamp| timestamp.quarter),
        )),
        Arc::new(Int64Array::new_null(row_count)),
        Arc::new(Int64Array::new_null(row_count)),
        Arc::new(StringArray::new_null(row_count)),
        Arc::new(Int64Array::new_null(row_count)),
        Arc::new(StringArray::new_null(row_count)),
        Arc::new(Int64Array::new_null(row_count)),
        Arc::new(Int64Array::new_null(row_count)),
    ]
}

fn vector_value(
    schema: &VectorIndexSchema,
    vectors: &FixedSizeListArray,
    row_index: usize,
) -> Result<Vec<f32>> {
    if vectors.is_null(row_index) {
        return Err(VectorError::InvalidDocument {
            index_name: schema.index_name.clone(),
            message: format!("vector value at row {row_index} is null"),
        });
    }
    let values = vectors.value(row_index);
    let values = values
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or_else(|| VectorError::InvalidDocument {
            index_name: schema.index_name.clone(),
            message: "vector values are not Float32".to_owned(),
        })?;
    let vector = (0..values.len())
        .map(|index| values.value(index))
        .collect::<Vec<_>>();
    if vector.len() != schema.vector_size {
        return Err(VectorError::InvalidDocument {
            index_name: schema.index_name.clone(),
            message: format!(
                "stored vector dimension expected {}, got {}",
                schema.vector_size,
                vector.len(),
            ),
        });
    }
    Ok(vector)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn schema(name: &str, vector_size: usize) -> VectorIndexSchema {
        VectorIndexSchema {
            index_name: name.to_owned(),
            id_field: "id".to_owned(),
            vector_field: "vector".to_owned(),
            vector_size,
        }
    }

    async fn connect_store(tempdir: &TempDir) -> LanceDbVectorStore {
        LanceDbVectorStore::connect(&VectorStoreConfig {
            db_uri: tempdir.path().to_string_lossy().to_string(),
            vector_size: 2,
            ..VectorStoreConfig::default()
        })
        .await
        .expect("store should connect")
    }

    #[tokio::test]
    async fn test_should_create_upsert_replace_and_reopen_lancedb_documents() {
        let tempdir = TempDir::new().expect("tempdir");
        let schema = schema("text_unit_text", 2);
        let store = connect_store(&tempdir).await;

        store.ensure_index(&schema).await.expect("ensure index");
        store
            .upsert_documents(
                &schema,
                &[
                    VectorDocument {
                        id: "a".to_owned(),
                        vector: vec![1.0, 0.0],
                    },
                    VectorDocument {
                        id: "b".to_owned(),
                        vector: vec![0.0, 1.0],
                    },
                ],
            )
            .await
            .expect("upsert");
        assert_eq!(store.count(&schema).await.expect("count"), 2);

        store
            .upsert_documents(
                &schema,
                &[VectorDocument {
                    id: "a".to_owned(),
                    vector: vec![0.5, 0.5],
                }],
            )
            .await
            .expect("replace");
        assert_eq!(store.count(&schema).await.expect("count"), 2);
        assert_eq!(
            store
                .get_by_id(&schema, "a")
                .await
                .expect("get")
                .expect("document")
                .vector,
            vec![0.5, 0.5]
        );

        let reopened = connect_store(&tempdir).await;
        assert_eq!(reopened.count(&schema).await.expect("count"), 2);
        assert_eq!(
            reopened
                .get_by_id(&schema, "b")
                .await
                .expect("get")
                .expect("document")
                .vector,
            vec![0.0, 1.0]
        );
    }

    #[tokio::test]
    async fn test_should_match_graphrag_schema_timestamps_and_vector_index() {
        let tempdir = TempDir::new().expect("tempdir");
        let schema = schema("text_unit_text", 2);
        let store = connect_store(&tempdir).await;
        store
            .upsert_documents(
                &schema,
                &[VectorDocument {
                    id: "document".to_owned(),
                    vector: vec![1.0, 0.0],
                }],
            )
            .await
            .expect("upsert");

        let table = store.open_table(&schema).await.expect("open table");
        let table_schema = table.schema().await.expect("table schema");
        assert_eq!(
            table_schema.as_ref(),
            arrow_schema(&schema).expect("expected schema").as_ref(),
        );
        assert_eq!(
            table_schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            vec![
                "id",
                "vector",
                "create_date",
                "update_date",
                "create_date_year",
                "create_date_month",
                "create_date_month_name",
                "create_date_day",
                "create_date_day_of_week",
                "create_date_hour",
                "create_date_quarter",
                "update_date_year",
                "update_date_month",
                "update_date_month_name",
                "update_date_day",
                "update_date_day_of_week",
                "update_date_hour",
                "update_date_quarter",
            ],
        );
        let indices = table.list_indices().await.expect("list indices");
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0].name, "vector_idx");
        assert_eq!(indices[0].index_type, IndexType::IvfFlat);
        assert_eq!(indices[0].columns, vec!["vector"]);

        let mut rows = table.query().execute().await.expect("query table");
        let batch = rows
            .try_next()
            .await
            .expect("read table")
            .expect("one record batch");
        assert_eq!(batch.num_rows(), 1);
        let create_dates = batch
            .column_by_name(CREATE_DATE_FIELD)
            .and_then(|column| column.as_any().downcast_ref::<StringArray>())
            .expect("create_date string column");
        let update_dates = batch
            .column_by_name(UPDATE_DATE_FIELD)
            .and_then(|column| column.as_any().downcast_ref::<StringArray>())
            .expect("update_date string column");
        let create_years = batch
            .column_by_name("create_date_year")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
            .expect("create_date_year int64 column");
        let timestamp = chrono::DateTime::parse_from_rfc3339(create_dates.value(0))
            .expect("GraphRAG-compatible ISO timestamp");
        assert_eq!(create_years.value(0), i64::from(timestamp.year()));
        assert!(update_dates.is_null(0));
        for name in timestamp_component_fields(UPDATE_DATE_FIELD)
            .iter()
            .map(Field::name)
        {
            assert!(
                batch
                    .column_by_name(name)
                    .expect("update component")
                    .is_null(0)
            );
        }
    }

    #[tokio::test]
    async fn test_should_reject_vector_dimension_mismatch() {
        let tempdir = TempDir::new().expect("tempdir");
        let schema = schema("entity_description", 2);
        let store = connect_store(&tempdir).await;

        let error = store
            .upsert_documents(
                &schema,
                &[VectorDocument {
                    id: "a".to_owned(),
                    vector: vec![1.0],
                }],
            )
            .await
            .expect_err("dimension mismatch should fail");
        assert!(error.to_string().contains("expected 2, got 1"));
    }

    #[tokio::test]
    async fn test_should_reject_non_finite_vectors() {
        let tempdir = TempDir::new().expect("tempdir");
        let schema = schema("entity_description", 2);
        let store = connect_store(&tempdir).await;
        store.ensure_index(&schema).await.expect("ensure index");

        for vector in [
            vec![f32::NAN, 0.0],
            vec![f32::INFINITY, 0.0],
            vec![f32::NEG_INFINITY, 0.0],
        ] {
            let error = store
                .upsert_documents(
                    &schema,
                    &[VectorDocument {
                        id: "bad-vector".to_owned(),
                        vector,
                    }],
                )
                .await
                .expect_err("non-finite vector should fail");

            assert!(error.to_string().contains("non-finite"));
            assert_eq!(store.count(&schema).await.expect("count"), 0);
        }
    }

    #[tokio::test]
    async fn test_should_reject_schema_mismatch() {
        let tempdir = TempDir::new().expect("tempdir");
        let store = connect_store(&tempdir).await;
        store
            .ensure_index(&schema("community_full_content", 2))
            .await
            .expect("ensure");

        let error = store
            .ensure_index(&schema("community_full_content", 3))
            .await
            .expect_err("schema mismatch should fail");
        assert!(error.to_string().contains("size 3"));
    }

    #[tokio::test]
    async fn test_should_isolate_multiple_embedding_indices() {
        let tempdir = TempDir::new().expect("tempdir");
        let store = connect_store(&tempdir).await;
        let first = schema("text_unit_text", 2);
        let second = schema("entity_description", 2);

        store
            .upsert_documents(
                &first,
                &[VectorDocument {
                    id: "same".to_owned(),
                    vector: vec![1.0, 0.0],
                }],
            )
            .await
            .expect("first");
        store
            .upsert_documents(
                &second,
                &[VectorDocument {
                    id: "same".to_owned(),
                    vector: vec![0.0, 1.0],
                }],
            )
            .await
            .expect("second");

        assert_eq!(store.count(&first).await.expect("first count"), 1);
        assert_eq!(store.count(&second).await.expect("second count"), 1);
        assert_eq!(
            store
                .get_by_id(&first, "same")
                .await
                .expect("get")
                .expect("document")
                .vector,
            vec![1.0, 0.0]
        );
        assert_eq!(
            store
                .get_by_id(&second, "same")
                .await
                .expect("get")
                .expect("document")
                .vector,
            vec![0.0, 1.0]
        );
    }

    #[tokio::test]
    async fn test_should_reset_only_selected_lancedb_index() {
        let tempdir = TempDir::new().expect("tempdir");
        let store = connect_store(&tempdir).await;
        let reset_schema = schema("custom_text_units", 2);
        let other_schema = schema("entity_description", 2);

        store
            .upsert_documents(
                &reset_schema,
                &[
                    VectorDocument {
                        id: "a".to_owned(),
                        vector: vec![1.0, 0.0],
                    },
                    VectorDocument {
                        id: "b".to_owned(),
                        vector: vec![0.0, 1.0],
                    },
                ],
            )
            .await
            .expect("seed reset index");
        store
            .upsert_documents(
                &other_schema,
                &[VectorDocument {
                    id: "other".to_owned(),
                    vector: vec![0.5, 0.5],
                }],
            )
            .await
            .expect("seed other index");
        assert_eq!(store.count(&reset_schema).await.expect("count"), 2);

        store
            .reset_index(&reset_schema)
            .await
            .expect("reset selected index");
        assert_eq!(store.count(&reset_schema).await.expect("reset count"), 0);
        assert_eq!(store.count(&other_schema).await.expect("other count"), 1);

        store
            .upsert_documents(
                &reset_schema,
                &[VectorDocument {
                    id: "c".to_owned(),
                    vector: vec![0.25, 0.75],
                }],
            )
            .await
            .expect("write after reset");

        let reopened = connect_store(&tempdir).await;
        assert_eq!(reopened.count(&reset_schema).await.expect("count"), 1);
        assert_eq!(
            reopened
                .get_by_id(&reset_schema, "c")
                .await
                .expect("get")
                .expect("document")
                .vector,
            vec![0.25, 0.75]
        );
        assert_eq!(reopened.count(&other_schema).await.expect("other count"), 1);
    }

    #[tokio::test]
    async fn test_should_search_top_k_in_distance_order_with_graphrag_scores() {
        let tempdir = TempDir::new().expect("tempdir");
        let schema = schema("text_unit_text", 2);
        let store = connect_store(&tempdir).await;
        store
            .upsert_documents(
                &schema,
                &[
                    VectorDocument {
                        id: "z-nearest".to_owned(),
                        vector: vec![0.0, 0.0],
                    },
                    VectorDocument {
                        id: "a-second".to_owned(),
                        vector: vec![1.0, 0.0],
                    },
                    VectorDocument {
                        id: "m-farthest".to_owned(),
                        vector: vec![3.0, 0.0],
                    },
                ],
            )
            .await
            .expect("seed vectors");

        let results = store
            .similarity_search_by_vector(&schema, &[0.2, 0.0], 2, true)
            .await
            .expect("search");

        assert_eq!(
            results
                .iter()
                .map(|result| result.document.id.as_str())
                .collect::<Vec<_>>(),
            vec!["z-nearest", "a-second"]
        );
        assert_eq!(results[0].document.vector, vec![0.0, 0.0]);
        assert!((results[0].score - 0.96).abs() < 1e-5);
        assert!((results[1].score - 0.36).abs() < 1e-5);
    }

    #[tokio::test]
    async fn test_should_omit_vectors_without_changing_search_order() {
        let tempdir = TempDir::new().expect("tempdir");
        let schema = schema("text_unit_text", 2);
        let store = connect_store(&tempdir).await;
        store
            .upsert_documents(
                &schema,
                &[
                    VectorDocument {
                        id: "b".to_owned(),
                        vector: vec![0.0, 0.0],
                    },
                    VectorDocument {
                        id: "a".to_owned(),
                        vector: vec![1.0, 0.0],
                    },
                ],
            )
            .await
            .expect("seed vectors");

        let results = store
            .similarity_search_by_vector(&schema, &[0.1, 0.0], 2, false)
            .await
            .expect("search");

        assert_eq!(results[0].document.id, "b");
        assert!(
            results
                .iter()
                .all(|result| result.document.vector.is_empty())
        );
    }

    #[tokio::test]
    async fn test_should_validate_similarity_query_before_provider_access() {
        let tempdir = TempDir::new().expect("tempdir");
        let schema = schema("text_unit_text", 2);
        let store = connect_store(&tempdir).await;

        for (vector, k, expected) in [
            (vec![0.0, 0.0], 0, "k must be greater than zero"),
            (Vec::new(), 1, "must not be empty"),
            (vec![0.0], 1, "expected 2, got 1"),
            (vec![f32::NAN, 0.0], 1, "non-finite"),
            (vec![f32::INFINITY, 0.0], 1, "non-finite"),
        ] {
            let error = store
                .similarity_search_by_vector(&schema, &vector, k, false)
                .await
                .expect_err("invalid query");
            assert!(error.to_string().contains(expected), "{error}");
        }

        let error = store
            .similarity_search_by_vector(&schema, &[0.0, 0.0], 1, false)
            .await
            .expect_err("missing index");
        assert!(matches!(error, VectorError::MissingIndex { .. }));
    }

    #[tokio::test]
    async fn test_should_search_empty_index_and_reject_wrong_schema() {
        let tempdir = TempDir::new().expect("tempdir");
        let expected_schema = schema("text_unit_text", 2);
        let store = connect_store(&tempdir).await;
        store
            .ensure_index(&expected_schema)
            .await
            .expect("empty index");
        assert!(
            store
                .similarity_search_by_vector(&expected_schema, &[0.0, 0.0], 10, false)
                .await
                .expect("empty search")
                .is_empty()
        );
        let wrong = schema("text_unit_text", 3);
        let error = store
            .similarity_search_by_vector(&wrong, &[0.0, 0.0, 0.0], 1, false)
            .await
            .expect_err("schema mismatch");
        assert!(error.to_string().contains("size 3"));

        store
            .upsert_documents(
                &expected_schema,
                &[VectorDocument {
                    id: "coexists".to_owned(),
                    vector: vec![0.5, 0.5],
                }],
            )
            .await
            .expect("upsert");
        assert!(
            store
                .get_by_id(&expected_schema, "coexists")
                .await
                .expect("get")
                .is_some()
        );
        assert_eq!(
            store
                .similarity_search_by_vector(&expected_schema, &[0.5, 0.5], 1, false)
                .await
                .expect("search")[0]
                .document
                .id,
            "coexists"
        );
    }
}
