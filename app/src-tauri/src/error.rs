use overtone_core::CoreError;
use serde::Serialize;
use ts_rs::TS;

/// Error returned by every Tauri command, serialized as
/// `{ "code": "...", "message": "..." }` for the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CommandError {
    /// Stable category the UI can branch on (see `CoreError::code`).
    pub code: String,
    /// Human-readable explanation to show the DJ.
    pub message: String,
}

impl CommandError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
        }
    }
}

impl From<&CoreError> for CommandError {
    fn from(err: &CoreError) -> Self {
        Self {
            code: err.code().into(),
            message: err.to_string(),
        }
    }
}
