//! How a scripted player makes out what is concealed.

use serde::{Deserialize, Serialize};

/// How a scripted player makes out what is concealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiDetection {
    /// Makes out a concealed entity only where its side's detection reaches —
    /// the fields its detectors project and the watches it holds.
    Detectors,
    /// Makes out every concealed entity, as if under its detection everywhere.
    Everywhere,
}
