//! Errors surfaced when running game scripts.

use thiserror::Error;

/// Something went wrong evaluating a script or reading what it produced.
#[derive(Debug, Clone, Error)]
pub enum ScriptError {
    #[error("ai error: {0}")]
    AiError(String),
    #[error("invalid command: {0}")]
    CommandError(String),
    #[error("content error: {0}")]
    ContentError(String),
    #[error("engine error: {0}")]
    EngineError(String),
    #[error("invalid number: {0}")]
    NumberError(String),
}
