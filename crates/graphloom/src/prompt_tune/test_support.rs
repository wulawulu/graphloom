//! Shared test utilities for prompt-tune tests.
//!
//! Compiled only under `#[cfg(test)]`.

use std::sync::Mutex;

use graphloom_llm::{CompletionModel, CompletionRequest, CompletionResponse};

/// A recording mock that captures every `CompletionRequest` and replies
/// with pre-configured responses in order.
///
/// Thread-safe and suitable for use with `tokio::spawn`.
#[derive(Debug)]
pub(crate) struct RecordingModel {
    pub(crate) responses: Mutex<Vec<String>>,
    pub(crate) requests: Mutex<Vec<CompletionRequest>>,
}

impl RecordingModel {
    pub(crate) fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl CompletionModel for RecordingModel {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<CompletionResponse, graphloom_llm::LlmError> {
        self.requests.lock().unwrap().push(request);
        let mut responses = self.responses.lock().unwrap();
        let content = if responses.is_empty() {
            String::new()
        } else {
            responses.remove(0)
        };
        Ok(CompletionResponse::text_for_test(
            "test.recording".to_owned(),
            content,
        ))
    }
}
