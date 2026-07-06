//! Text-file input reader.

use std::{collections::VecDeque, path::Path, sync::Arc};

use futures_util::stream;
use graphloom_storage::{FileStorage, Storage};
use regex::Regex;
use tracing::warn;

use crate::{DocumentStream, InputError, InputReader, Result, TextDocument, gen_sha512_hash};

const DEFAULT_TEXT_FILE_PATTERN: &str = r".*\.txt$";

/// Reader implementation for text files.
#[derive(Debug, Clone)]
pub struct TextFileReader {
    storage: Arc<dyn Storage>,
    file_pattern: Regex,
}

impl TextFileReader {
    /// Create a text-file reader with `GraphRAG`'s default `.*\.txt$` pattern.
    ///
    /// # Errors
    ///
    /// Returns an error if the built-in pattern cannot be compiled.
    pub fn new(storage: Arc<dyn Storage>) -> Result<Self> {
        Self::with_file_pattern(storage, DEFAULT_TEXT_FILE_PATTERN)
    }

    /// Create a text-file reader with an explicit regular expression.
    ///
    /// # Errors
    ///
    /// Returns an error when `file_pattern` is not a valid regex.
    pub fn with_file_pattern(storage: Arc<dyn Storage>, file_pattern: &str) -> Result<Self> {
        let file_pattern =
            Regex::new(file_pattern).map_err(|source| InputError::InvalidPattern {
                pattern: file_pattern.to_owned(),
                source,
            })?;
        Ok(Self {
            storage,
            file_pattern,
        })
    }

    /// Read a single text file into documents.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage object cannot be read or is not UTF-8.
    pub async fn read_file(&self, path: &str) -> Result<Vec<TextDocument>> {
        read_text_file(Arc::clone(&self.storage), path).await
    }
}

impl InputReader for TextFileReader {
    fn read_documents(&self) -> DocumentStream<'_> {
        let state = ReaderState {
            storage: Arc::clone(&self.storage),
            file_pattern: self.file_pattern.clone(),
            files: None,
            file_index: 0,
            buffered_documents: VecDeque::new(),
        };

        Box::pin(stream::try_unfold(state, |mut state| async move {
            if state.files.is_none() {
                let files = state
                    .storage
                    .list("")
                    .await?
                    .into_iter()
                    .filter(|file| state.file_pattern.is_match(file))
                    .collect::<Vec<_>>();
                state.files = Some(files);
            }

            loop {
                if let Some(document) = state.buffered_documents.pop_front() {
                    return Ok(Some((document, state)));
                }

                let Some(files) = state.files.as_ref() else {
                    return Ok(None);
                };
                let Some(file) = files.get(state.file_index).cloned() else {
                    return Ok(None);
                };
                state.file_index = state.file_index.saturating_add(1);

                match read_text_file(Arc::clone(&state.storage), &file).await {
                    Ok(documents) => state.buffered_documents = documents.into(),
                    Err(error) => {
                        warn!(file = %file, error = %error, "skipping unreadable input file");
                    }
                }
            }
        }))
    }
}

#[derive(Debug)]
struct ReaderState {
    storage: Arc<dyn Storage>,
    file_pattern: Regex,
    files: Option<Vec<String>>,
    file_index: usize,
    buffered_documents: VecDeque<TextDocument>,
}

async fn read_text_file(storage: Arc<dyn Storage>, path: &str) -> Result<Vec<TextDocument>> {
    let bytes = storage
        .get(path)
        .await?
        .ok_or_else(|| InputError::MissingInput {
            path: path.to_owned(),
        })?;
    let text = String::from_utf8(bytes).map_err(|source| InputError::Utf8 {
        path: path.to_owned(),
        source,
    })?;
    let title = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_owned();
    let creation_date = storage.get_creation_date(path).await?;

    Ok(vec![TextDocument {
        id: gen_sha512_hash([text.as_str()]),
        text,
        title,
        creation_date,
        raw_data: None,
    }])
}

/// Filesystem convenience wrapper for [`TextFileReader`].
#[derive(Debug, Clone)]
pub struct FileInputReader {
    inner: TextFileReader,
}

impl FileInputReader {
    /// Create a filesystem-backed reader rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage root or default file pattern is invalid.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let storage = Arc::new(FileStorage::new(root)?);
        Ok(Self {
            inner: TextFileReader::new(storage)?,
        })
    }

    /// Create a filesystem-backed reader with an explicit regular expression.
    ///
    /// # Errors
    ///
    /// Returns an error when storage creation or regex compilation fails.
    pub fn with_file_pattern(root: impl AsRef<Path>, file_pattern: &str) -> Result<Self> {
        let storage = Arc::new(FileStorage::new(root)?);
        Ok(Self {
            inner: TextFileReader::with_file_pattern(storage, file_pattern)?,
        })
    }
}

impl InputReader for FileInputReader {
    fn read_documents(&self) -> DocumentStream<'_> {
        self.inner.read_documents()
    }
}
