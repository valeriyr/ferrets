//! Errors raised handling the wire format.

/// Something went wrong with a message on the wire.
#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("bcs error: {0}")]
    BcsError(#[from] bcs::Error),
}
