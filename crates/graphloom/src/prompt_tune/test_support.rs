//! Shared test utilities for prompt-tune tests.
//!
//! Compiled only under `#[cfg(test)]`.

use std::sync::Mutex;

use graphloom_llm::{CompletionModel, CompletionRequest, CompletionResponse, LlmError};

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

/// One exact request/response pair consumed by [`PromptTuneReplayModel`].
#[derive(Debug)]
pub(crate) struct PromptTuneReplayRecord {
    request: CompletionRequest,
    response: String,
    consumed: bool,
}

impl PromptTuneReplayRecord {
    pub(crate) fn new(request: CompletionRequest, response: impl Into<String>) -> Self {
        Self {
            request,
            response: response.into(),
            consumed: false,
        }
    }
}

/// Request-aware replay model for concurrent prompt-tune tests.
///
/// Records match the complete canonical request, including every message role
/// and byte of content. A request must match exactly one unconsumed record.
#[derive(Debug)]
pub(crate) struct PromptTuneReplayModel {
    records: Mutex<Vec<PromptTuneReplayRecord>>,
}

impl PromptTuneReplayModel {
    pub(crate) fn new(records: Vec<PromptTuneReplayRecord>) -> Self {
        Self {
            records: Mutex::new(records),
        }
    }

    pub(crate) fn assert_exhausted(&self) {
        let records = self.records.lock().unwrap();
        let remaining = records.iter().filter(|record| !record.consumed).count();
        assert_eq!(
            remaining, 0,
            "{remaining} replay response(s) were not consumed"
        );
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

#[async_trait::async_trait]
impl CompletionModel for PromptTuneReplayModel {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<CompletionResponse, LlmError> {
        let mut records = self.records.lock().unwrap();
        let matches = records
            .iter()
            .enumerate()
            .filter(|(_, record)| !record.consumed && record.request == request)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        let Some(index) = matches.first().copied() else {
            return Err(LlmError::InvalidRequest {
                operation: "prompt_tune.replay",
                message: "no unconsumed exact request match".to_owned(),
            });
        };
        let response = records[index].response.clone();
        if matches
            .iter()
            .any(|candidate| records[*candidate].response != response)
        {
            return Err(LlmError::InvalidRequest {
                operation: "prompt_tune.replay",
                message: format!(
                    "{} identical requests map to different responses",
                    matches.len()
                ),
            });
        }
        let record = &mut records[index];
        record.consumed = true;

        Ok(CompletionResponse::text_for_test(
            "test.prompt-tune-replay",
            record.response.clone(),
        ))
    }
}
