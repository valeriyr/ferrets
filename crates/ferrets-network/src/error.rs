//! The crate's top-level error type.

/// Something went wrong in the network layer.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("timeout error: {0}")]
    TimeoutError(String),
    #[error("transport error: {0}")]
    TransportError(#[from] crate::transport::error::TransportError),
    #[error("message error: {0}")]
    MessageError(#[from] crate::message::error::MessageError),
    #[error("unsupported error: {0}")]
    UnsupportedError(String),
}
