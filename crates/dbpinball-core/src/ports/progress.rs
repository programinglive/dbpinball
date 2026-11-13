use serde::Serialize;

/// A coarse phase marker for a long-running operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Connecting,
    Inspecting,
    Done,
}

/// A progress event a presentation layer may render.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub phase: Phase,
    pub message: String,
}

impl ProgressEvent {
    pub fn new(phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
        }
    }
}

/// Receives progress events from a service.
///
/// The desktop adapter forwards these as Tauri events; a CLI may print them to
/// stderr. Services never know which.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: ProgressEvent);
}

/// A sink that discards every event. Useful for tests and headless callers.
pub struct NoopProgress;

impl ProgressSink for NoopProgress {
    fn emit(&self, _event: ProgressEvent) {}
}
