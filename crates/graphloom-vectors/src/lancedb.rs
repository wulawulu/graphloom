//! `LanceDB` vector store provider.

use std::{collections::BTreeSet, fmt, sync::Arc};

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, RecordBatchReader,
    StringArray, types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use lancedb::{
    Connection, Table, connect,
    query::{ExecutableQuery, QueryBase, Select},
};

use crate::{
    Result, VectorDocument, VectorError, VectorIndexSchema, VectorStore, VectorStoreConfig,
};

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
}

#[async_trait]
impl VectorStore for LanceDbVectorStore {
    async fn ensure_index(&self, schema: &VectorIndexSchema) -> Result<()> {
        schema.validate()?;
        if self.table_exists(&schema.index_name).await? {
            let table = self.open_table(schema).await?;
            validate_table_schema(schema, table.schema().await?.as_ref())?;
            return Ok(());
        }

        self.connection
            .create_empty_table(&schema.index_name, arrow_schema(schema)?)
            .execute()
            .await?;
        Ok(())
    }

    async fn reset_index(&self, schema: &VectorIndexSchema) -> Result<()> {
        schema.validate()?;
        if self.table_exists(&schema.index_name).await? {
            self.connection.drop_table(&schema.index_name, &[]).await?;
        }
        self.connection
            .create_empty_table(&schema.index_name, arrow_schema(schema)?)
            .execute()
            .await?;
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

        let mut upsert = table.merge_insert(&[schema.id_field.as_str()]);
        upsert
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        upsert.execute(reader).await?;
        Ok(())
    }

    async fn count(&self, schema: &VectorIndexSchema) -> Result<usize> {
        schema.validate()?;
        let table = self.open_table(schema).await?;
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
        let table = self.open_table(schema).await?;
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
}

fn arrow_schema(schema: &VectorIndexSchema) -> Result<SchemaRef> {
    let vector_size =
        i32::try_from(schema.vector_size).map_err(|source| VectorError::InvalidConfig {
            message: format!(
                "vector_size {} does not fit i32: {source}",
                schema.vector_size
            ),
        })?;
    Ok(Arc::new(Schema::new(vec![
        Field::new(&schema.id_field, DataType::Utf8, false),
        Field::new(
            &schema.vector_field,
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                vector_size,
            ),
            false,
        ),
    ])))
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
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(ids), Arc::new(vectors)],
    )?;
    Ok(Box::new(RecordBatchIterator::new(
        vec![Ok(batch)],
        arrow_schema,
    )))
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
}
