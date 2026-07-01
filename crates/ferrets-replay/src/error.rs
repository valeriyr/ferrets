//! Errors surfaced when recording or loading a replay.

use thiserror::Error;

/// Something went wrong reading, writing, or (de)serializing replay data.
#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("not a ferrets replay")]
    BadMagic,
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("bcs error: {0}")]
    BcsError(#[from] bcs::Error),
    #[error("unsupported replay format version {found}; this build reads version {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },
}
