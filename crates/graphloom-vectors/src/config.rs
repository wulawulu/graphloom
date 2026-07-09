//! Vector store configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Result, VectorError};

const DEFAULT_DB_URI: &str = "output/lancedb";
const DEFAULT_VECTOR_SIZE: usize = 3_072;
const DEFAULT_ID_FIELD: &str = "id";
const DEFAULT_VECTOR_FIELD: &str = "vector";

/// Vector store provider type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum VectorStoreType {
    /// Local or remote `LanceDB` store.
    #[default]
    #[serde(rename = "lancedb")]
    LanceDb,
}

impl<'de> Deserialize<'de> for VectorStoreType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "lancedb" | "lance_db" => Ok(Self::LanceDb),
            other => Err(serde::de::Error::custom(format!(
                "unsupported vector store type {other}; only lancedb is supported",
            ))),
        }
    }
}

/// Per-index vector schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct VectorIndexSchema {
    /// `LanceDB` table/index name.
    #[serde(alias = "index_name")]
    pub index_name: String,
    /// Document id field.
    #[serde(alias = "id_field")]
    pub id_field: String,
    /// Vector field.
    #[serde(alias = "vector_field")]
    pub vector_field: String,
    /// Expected vector dimensionality.
    #[serde(alias = "vector_size")]
    pub vector_size: usize,
}

impl Default for VectorIndexSchema {
    fn default() -> Self {
        Self {
            index_name: String::new(),
            id_field: DEFAULT_ID_FIELD.to_owned(),
            vector_field: DEFAULT_VECTOR_FIELD.to_owned(),
            vector_size: 0,
        }
    }
}

impl VectorIndexSchema {
    /// Build default schema for an embedding name.
    #[must_use]
    pub fn for_embedding_name(name: &str, vector_size: usize) -> Self {
        Self {
            index_name: name.to_owned(),
            id_field: DEFAULT_ID_FIELD.to_owned(),
            vector_field: DEFAULT_VECTOR_FIELD.to_owned(),
            vector_size,
        }
    }

    /// Validate this schema.
    ///
    /// # Errors
    ///
    /// Returns an error if identifiers are unsafe or vector size is zero.
    pub fn validate(&self) -> Result<()> {
        validate_identifier("index_name", &self.index_name)?;
        validate_identifier("id_field", &self.id_field)?;
        validate_identifier("vector_field", &self.vector_field)?;
        if self.vector_size == 0 {
            return Err(VectorError::InvalidConfig {
                message: format!(
                    "index {} vector_size must be greater than zero",
                    self.index_name
                ),
            });
        }
        Ok(())
    }
}

/// Vector store configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct VectorStoreConfig {
    /// Provider type.
    #[serde(rename = "type", alias = "store_type")]
    pub store_type: VectorStoreType,
    /// `LanceDB` URI.
    #[serde(alias = "db_uri")]
    pub db_uri: String,
    /// Default vector size for indices that do not override it.
    #[serde(alias = "vector_size")]
    pub vector_size: usize,
    /// Optional per-embedding index schemas.
    #[serde(alias = "index_schema")]
    pub index_schema: BTreeMap<String, VectorIndexSchema>,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            store_type: VectorStoreType::LanceDb,
            db_uri: DEFAULT_DB_URI.to_owned(),
            vector_size: DEFAULT_VECTOR_SIZE,
            index_schema: BTreeMap::new(),
        }
    }
}

impl VectorStoreConfig {
    /// Resolve an index schema for an embedding name.
    #[must_use]
    pub fn schema_for(&self, embedding_name: &str) -> VectorIndexSchema {
        let mut schema = self
            .index_schema
            .get(embedding_name)
            .cloned()
            .unwrap_or_else(|| {
                VectorIndexSchema::for_embedding_name(embedding_name, self.vector_size)
            });
        if schema.index_name.is_empty() {
            embedding_name.clone_into(&mut schema.index_name);
        }
        if schema.id_field.is_empty() {
            DEFAULT_ID_FIELD.clone_into(&mut schema.id_field);
        }
        if schema.vector_field.is_empty() {
            DEFAULT_VECTOR_FIELD.clone_into(&mut schema.vector_field);
        }
        if schema.vector_size == 0 {
            schema.vector_size = self.vector_size;
        }
        schema
    }

    /// Validate this configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the provider, URI, vector size, or schema names are invalid.
    pub fn validate(&self) -> Result<()> {
        if self.db_uri.trim().is_empty() {
            return Err(VectorError::InvalidConfig {
                message: "db_uri must not be empty".to_owned(),
            });
        }
        if self.vector_size == 0 {
            return Err(VectorError::InvalidConfig {
                message: "vector_size must be greater than zero".to_owned(),
            });
        }
        for embedding_name in self.index_schema.keys() {
            validate_identifier("embedding name", embedding_name)?;
            let resolved = self.schema_for(embedding_name);
            resolved.validate()?;
        }
        Ok(())
    }
}

/// Validate a `LanceDB` identifier used as a table or column name.
///
/// # Errors
///
/// Returns an error when `value` is not a simple identifier.
pub fn validate_identifier(kind: &str, value: &str) -> Result<()> {
    let mut chars = value.chars();
    let first_is_valid = chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic());
    let rest_is_valid = chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
    if first_is_valid && rest_is_valid {
        Ok(())
    } else {
        Err(VectorError::InvalidConfig {
            message: format!("{kind} {value:?} must match ^[A-Za-z_][A-Za-z0-9_]*$"),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_should_default_lancedb_schema_for_embedding_name() {
        let config = VectorStoreConfig {
            vector_size: 8,
            ..VectorStoreConfig::default()
        };
        let schema = config.schema_for("text_unit_text");

        assert_eq!(schema.index_name, "text_unit_text");
        assert_eq!(schema.id_field, "id");
        assert_eq!(schema.vector_field, "vector");
        assert_eq!(schema.vector_size, 8);
        config.validate().expect("default config should validate");
    }

    #[test]
    fn test_should_inherit_global_vector_size_for_partial_index_schema() {
        let config: VectorStoreConfig = serde_yaml::from_str(
            r"
type: lancedb
dbUri: output/lancedb
vectorSize: 1024
indexSchema:
  entity_description:
    indexName: custom_entities
",
        )
        .expect("config should deserialize");

        let schema = config.schema_for("entity_description");
        assert_eq!(schema.index_name, "custom_entities");
        assert_eq!(schema.vector_size, 1024);
        config.validate().expect("config should validate");
    }

    #[test]
    fn test_should_allow_per_index_vector_size_override() {
        let config: VectorStoreConfig = serde_yaml::from_str(
            r"
type: lancedb
dbUri: output/lancedb
vectorSize: 1024
indexSchema:
  entity_description:
    vectorSize: 1536
",
        )
        .expect("config should deserialize");

        assert_eq!(config.schema_for("entity_description").vector_size, 1536);
        assert_eq!(config.schema_for("text_unit_text").vector_size, 1024);
    }

    #[test]
    fn test_should_reject_zero_global_vector_size() {
        let config = VectorStoreConfig {
            vector_size: 0,
            ..VectorStoreConfig::default()
        };

        let error = config.validate().expect_err("zero global vector size");
        assert!(error.to_string().contains("vector_size"));
    }

    #[test]
    fn test_should_serde_lancedb_provider_round_trip() {
        let original = VectorStoreConfig::default();
        let json = serde_json::to_string(&original).expect("json serialize");
        assert!(json.contains(r#""type":"lancedb""#));
        let from_json: VectorStoreConfig = serde_json::from_str(&json).expect("json deserialize");
        assert_eq!(from_json, original);

        let yaml = serde_yaml::to_string(&original).expect("yaml serialize");
        assert!(yaml.contains("type: lancedb"));
        let from_yaml: VectorStoreConfig = serde_yaml::from_str(&yaml).expect("yaml deserialize");
        assert_eq!(from_yaml, original);
    }

    #[test]
    fn test_should_reject_unsupported_vector_store_type() {
        let error = serde_json::from_value::<VectorStoreConfig>(json!({
            "type": "memory",
        }))
        .expect_err("memory vector store must not deserialize");

        assert!(error.to_string().contains("only lancedb is supported"));

        let error = serde_json::from_value::<VectorStoreConfig>(json!({
            "type": "unknown",
        }))
        .expect_err("unknown vector store must not deserialize");
        assert!(error.to_string().contains("only lancedb is supported"));
    }

    #[test]
    fn test_should_reject_unsafe_index_and_field_identifiers() {
        assert!(validate_identifier("index", "valid_name_1").is_ok());
        assert!(validate_identifier("index", "1invalid").is_err());
        assert!(validate_identifier("index", "bad-name").is_err());
        assert!(validate_identifier("index", "bad name").is_err());
    }
}
