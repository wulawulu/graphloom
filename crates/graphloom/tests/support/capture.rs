#![allow(dead_code)]

//! Minimal scoped `tracing` capture layer for observability contract tests.
//!
//! The layer records span creation, parent IDs, field records, named events,
//! and span close in a structured form. Tests never parse formatter text.

use std::{
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
};

/// One captured span.
#[derive(Debug, Clone)]
pub(crate) struct CapturedSpan {
    pub(crate) id: Id,
    pub(crate) name: String,
    pub(crate) parent: Option<Id>,
    pub(crate) fields: Vec<(String, String)>,
    pub(crate) closed: bool,
}

/// One captured event, including its stable name and fields.
#[derive(Debug, Clone)]
pub(crate) struct CapturedEvent {
    pub(crate) name: String,
    pub(crate) fields: Vec<(String, String)>,
}

/// Thread-safe capture state shared with the layer.
#[derive(Debug, Clone, Default)]
pub(crate) struct CaptureState {
    pub(crate) spans: Vec<CapturedSpan>,
    pub(crate) events: Vec<CapturedEvent>,
}

/// `tracing_subscriber` layer that records structured span/event data.
#[derive(Debug, Clone)]
pub(crate) struct CaptureLayer {
    state: Arc<Mutex<CaptureState>>,
}

impl CaptureLayer {
    /// Create a layer writing into the shared state.
    pub(crate) fn new(state: Arc<Mutex<CaptureState>>) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Vec::new();
        attributes.values().record(&mut FieldRecorder {
            fields: &mut fields,
        });
        let parent = attributes
            .parent()
            .cloned()
            .or_else(|| ctx.lookup_current().map(|span| span.id().clone()));
        self.state
            .lock()
            .expect("capture state lock")
            .spans
            .push(CapturedSpan {
                id: id.clone(),
                name: attributes.metadata().name().to_owned(),
                parent,
                fields,
                closed: false,
            });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut fields = Vec::new();
        values.record(&mut FieldRecorder {
            fields: &mut fields,
        });
        let mut state = self.state.lock().expect("capture state lock");
        if let Some(span) = state.spans.iter_mut().find(|span| span.id == *id) {
            span.fields.extend(fields);
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = Vec::new();
        event.record(&mut FieldRecorder {
            fields: &mut fields,
        });
        self.state
            .lock()
            .expect("capture state lock")
            .events
            .push(CapturedEvent {
                name: event.metadata().name().to_owned(),
                fields,
            });
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        let mut state = self.state.lock().expect("capture state lock");
        if let Some(span) = state.spans.iter_mut().find(|span| span.id == id) {
            span.closed = true;
        }
    }
}

/// Structured field visitor formatting every value with `Debug`.
struct FieldRecorder<'a> {
    fields: &'a mut Vec<(String, String)>,
}

impl Visit for FieldRecorder<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .push((field.name().to_owned(), format!("{value:?}")));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn Error + 'static)) {
        self.fields
            .push((field.name().to_owned(), format!("{value:?}")));
    }
}

impl CapturedSpan {
    /// Return the last recorded value for a field name.
    pub(crate) fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .rev()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

impl CapturedEvent {
    /// Return the first recorded value for a field name.
    pub(crate) fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Create a scoped registry subscriber with a capture layer.
pub(crate) fn capture_subscriber(
    state: Arc<Mutex<CaptureState>>,
) -> impl Subscriber + Send + Sync + 'static {
    tracing_subscriber::registry().with(CaptureLayer::new(state))
}
