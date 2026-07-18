//! Test-only logical vector interoperability bridge.
//!
//! This example deliberately sits outside `GraphLoom`'s production CLI and Query
//! path. It exports and imports the logical `id` plus `vector` records through
//! the public [`VectorStore`] contract so compatibility tests can bridge
//! different `LanceDB` on-disk versions without regenerating embeddings.

use std::{
    collections::BTreeSet,
    env,
    path::{Path, PathBuf},
};

use graphloom_vectors::{
    LanceDbVectorStore, VectorDocument, VectorError, VectorIndexSchema, VectorStore,
    VectorStoreConfig,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const FORMAT_VERSION: u32 = 1;
const COLLECTION_NAMES: [&str; 3] = [
    "community_full_content",
    "entity_description",
    "text_unit_text",
];

#[derive(Debug, Error)]
enum ManifestError {
    #[error("usage: compat_vector_manifest <export|import|inspect> <arguments>")]
    Usage,
    #[error("invalid vector dimension {value:?}; expected a positive integer")]
    InvalidDimension { value: String },
    #[error("manifest validation failed: {message}")]
    InvalidManifest { message: String },
    #[error("import target must be a new or empty directory: {path}")]
    NonEmptyImportTarget { path: PathBuf },
    #[error("failed to read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode or encode {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Vector(#[from] VectorError),
}

type Result<T> = std::result::Result<T, ManifestError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorManifest {
    format_version: u32,
    collections: Vec<ManifestCollection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestCollection {
    name: String,
    dimension: usize,
    records: Vec<ManifestRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestRecord {
    id: String,
    vector: Vec<f32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [command, db_uri, manifest_path, dimension] if command == "export" => {
            let dimension = parse_dimension(dimension)?;
            let manifest = export_manifest(db_uri, dimension).await?;
            write_manifest(Path::new(manifest_path), &manifest).await
        }
        [command, manifest_path, db_uri] if command == "import" => {
            let manifest = read_manifest(Path::new(manifest_path)).await?;
            validate_import_target(Path::new(db_uri)).await?;
            import_manifest(db_uri, &manifest).await
        }
        [command, db_uri, dimension] if command == "inspect" => {
            let dimension = parse_dimension(dimension)?;
            let manifest = export_manifest(db_uri, dimension).await?;
            let output =
                serde_json::to_string_pretty(&manifest).map_err(|source| ManifestError::Json {
                    path: PathBuf::from("<stdout>"),
                    source,
                })?;
            println!("{output}");
            Ok(())
        }
        _ => Err(ManifestError::Usage),
    }
}

fn parse_dimension(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|dimension| *dimension > 0)
        .ok_or_else(|| ManifestError::InvalidDimension {
            value: value.to_owned(),
        })
}

async fn export_manifest(db_uri: &str, dimension: usize) -> Result<VectorManifest> {
    let store = connect_store(db_uri, dimension).await?;
    let mut collections = Vec::with_capacity(COLLECTION_NAMES.len());
    for name in COLLECTION_NAMES {
        let schema = VectorIndexSchema::for_embedding_name(name, dimension);
        let ids = store.ids(&schema).await?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let document = store.get_by_id(&schema, &id).await?.ok_or_else(|| {
                ManifestError::InvalidManifest {
                    message: format!("{name} listed id {id:?} but could not read it"),
                }
            })?;
            records.push(ManifestRecord {
                id: document.id,
                vector: document.vector,
            });
        }
        validate_ann_semantics(&store, &schema, &records).await?;
        collections.push(ManifestCollection {
            name: name.to_owned(),
            dimension,
            records,
        });
    }
    let manifest = VectorManifest {
        format_version: FORMAT_VERSION,
        collections,
    };
    validate_manifest(&manifest)?;
    Ok(manifest)
}

async fn validate_ann_semantics(
    store: &LanceDbVectorStore,
    schema: &VectorIndexSchema,
    records: &[ManifestRecord],
) -> Result<()> {
    let Some(expected) = records.first() else {
        return Ok(());
    };
    let results = store
        .similarity_search_by_vector(schema, &expected.vector, records.len().min(3), true)
        .await?;
    if !results
        .iter()
        .any(|result| result.document.id == expected.id)
    {
        return Err(ManifestError::InvalidManifest {
            message: format!(
                "{} ANN results do not contain probe id {:?}",
                schema.index_name, expected.id
            ),
        });
    }
    let exact = results
        .iter()
        .find(|result| result.document.id == expected.id)
        .ok_or_else(|| ManifestError::InvalidManifest {
            message: format!("{} ANN probe result disappeared", schema.index_name),
        })?;
    if (exact.score - 1.0).abs() > f32::EPSILON || exact.document.vector != expected.vector {
        return Err(ManifestError::InvalidManifest {
            message: format!(
                "{} ANN score or vector differs for probe id {:?}",
                schema.index_name, expected.id
            ),
        });
    }
    Ok(())
}

async fn import_manifest(db_uri: &str, manifest: &VectorManifest) -> Result<()> {
    validate_manifest(manifest)?;
    let dimension = manifest
        .collections
        .first()
        .map(|collection| collection.dimension)
        .ok_or_else(|| ManifestError::InvalidManifest {
            message: "collections must not be empty".to_owned(),
        })?;
    let store = connect_store(db_uri, dimension).await?;
    for collection in &manifest.collections {
        let schema = VectorIndexSchema::for_embedding_name(&collection.name, collection.dimension);
        store.ensure_index(&schema).await?;
        let documents = collection
            .records
            .iter()
            .map(|record| VectorDocument {
                id: record.id.clone(),
                vector: record.vector.clone(),
            })
            .collect::<Vec<_>>();
        store.upsert_documents(&schema, &documents).await?;
        if store.count(&schema).await? != documents.len() {
            return Err(ManifestError::InvalidManifest {
                message: format!(
                    "{} import count does not equal manifest count",
                    collection.name
                ),
            });
        }
    }
    Ok(())
}

async fn connect_store(db_uri: &str, dimension: usize) -> Result<LanceDbVectorStore> {
    let mut config = VectorStoreConfig::default();
    config.db_uri = db_uri.to_owned();
    config.vector_size = dimension;
    Ok(LanceDbVectorStore::connect(&config).await?)
}

fn validate_manifest(manifest: &VectorManifest) -> Result<()> {
    if manifest.format_version != FORMAT_VERSION {
        return Err(ManifestError::InvalidManifest {
            message: format!(
                "unsupported format_version {}; expected {FORMAT_VERSION}",
                manifest.format_version
            ),
        });
    }
    let names = manifest
        .collections
        .iter()
        .map(|collection| collection.name.as_str())
        .collect::<BTreeSet<_>>();
    let expected = COLLECTION_NAMES.into_iter().collect::<BTreeSet<_>>();
    if names != expected || manifest.collections.len() != expected.len() {
        return Err(ManifestError::InvalidManifest {
            message: format!(
                "collections must be exactly {}",
                COLLECTION_NAMES.join(", ")
            ),
        });
    }
    for collection in &manifest.collections {
        if collection.dimension == 0 {
            return Err(ManifestError::InvalidManifest {
                message: format!("{} dimension must be positive", collection.name),
            });
        }
        let mut previous_id: Option<&str> = None;
        for record in &collection.records {
            if record.id.is_empty() {
                return Err(ManifestError::InvalidManifest {
                    message: format!("{} contains an empty id", collection.name),
                });
            }
            if previous_id.is_some_and(|previous| previous >= record.id.as_str()) {
                return Err(ManifestError::InvalidManifest {
                    message: format!(
                        "{} records must have unique ids in ascending order",
                        collection.name
                    ),
                });
            }
            if record.vector.len() != collection.dimension {
                return Err(ManifestError::InvalidManifest {
                    message: format!(
                        "{} record {:?} has dimension {}, expected {}",
                        collection.name,
                        record.id,
                        record.vector.len(),
                        collection.dimension
                    ),
                });
            }
            if record.vector.iter().any(|value| !value.is_finite()) {
                return Err(ManifestError::InvalidManifest {
                    message: format!(
                        "{} record {:?} contains a non-finite vector value",
                        collection.name, record.id
                    ),
                });
            }
            previous_id = Some(&record.id);
        }
    }
    Ok(())
}

async fn validate_import_target(path: &Path) -> Result<()> {
    if !tokio::fs::try_exists(path)
        .await
        .map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?
    {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if entries
        .next_entry()
        .await
        .map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .is_some()
    {
        return Err(ManifestError::NonEmptyImportTarget {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

async fn read_manifest(path: &Path) -> Result<VectorManifest> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let manifest = serde_json::from_slice(&bytes).map_err(|source| ManifestError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

async fn write_manifest(path: &Path, manifest: &VectorManifest) -> Result<()> {
    validate_manifest(manifest)?;
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|source| ManifestError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    tokio::fs::write(path, bytes)
        .await
        .map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> VectorManifest {
        VectorManifest {
            format_version: FORMAT_VERSION,
            collections: COLLECTION_NAMES
                .iter()
                .map(|name| ManifestCollection {
                    name: (*name).to_owned(),
                    dimension: 2,
                    records: vec![ManifestRecord {
                        id: "record-1".to_owned(),
                        vector: vec![0.25, 0.75],
                    }],
                })
                .collect(),
        }
    }

    #[test]
    fn test_should_accept_versioned_sorted_finite_manifest() {
        assert!(validate_manifest(&valid_manifest()).is_ok());
    }

    #[test]
    fn test_should_reject_unknown_collection() {
        let mut manifest = valid_manifest();
        manifest.collections[0].name = "unknown".to_owned();
        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::InvalidManifest { .. })
        ));
    }

    #[test]
    fn test_should_reject_duplicate_or_unsorted_ids() {
        let mut manifest = valid_manifest();
        manifest.collections[0].records.push(ManifestRecord {
            id: "record-1".to_owned(),
            vector: vec![0.25, 0.75],
        });
        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::InvalidManifest { .. })
        ));
    }

    #[test]
    fn test_should_reject_non_finite_or_wrong_dimension_vectors() {
        let mut non_finite = valid_manifest();
        non_finite.collections[0].records[0].vector[0] = f32::NAN;
        assert!(validate_manifest(&non_finite).is_err());

        let mut wrong_dimension = valid_manifest();
        wrong_dimension.collections[0].records[0].vector.pop();
        assert!(validate_manifest(&wrong_dimension).is_err());
    }

    #[test]
    fn test_should_reject_unknown_manifest_fields() {
        let mut value = serde_json::to_value(valid_manifest()).expect("serialize manifest");
        value
            .as_object_mut()
            .expect("manifest is an object")
            .insert("unknown".to_owned(), serde_json::Value::Bool(true));

        assert!(serde_json::from_value::<VectorManifest>(value).is_err());
    }
}
