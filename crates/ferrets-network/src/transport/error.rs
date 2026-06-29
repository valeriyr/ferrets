//! Errors a transport raises moving bytes between peers.

/// Something went wrong moving bytes between peers.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("internal error: {0}")]
    InternalError(String),
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}
