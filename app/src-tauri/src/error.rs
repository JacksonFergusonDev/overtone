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
    /// Another job of the same kind is already running.
    pub fn busy(message: impl Into<String>) -> Self {
        Self {
            code: "busy".into(),
            message: message.into(),
        }
    }

    /// A file or folder couldn't be read or written.
    pub fn io(message: impl Into<String>) -> Self {
        Self {
            code: "io".into(),
            message: message.into(),
        }
    }

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

impl From<CoreError> for CommandError {
    fn from(err: CoreError) -> Self {
        Self::from(&err)
    }
}

impl From<overtone_core::library::LibraryError> for CommandError {
    fn from(err: overtone_core::library::LibraryError) -> Self {
        Self::from(CoreError::from(err))
    }
}
