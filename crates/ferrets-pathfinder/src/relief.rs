//! A cell's relief on a layer.

use serde::{Deserialize, Serialize};

use crate::slope::Slope;

/// How a layer lies at one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relief {
    /// The layer's level at the cell.
    pub level: u32,
    /// The slope the cell is part of on the layer, or `None` on flat ground.
    pub slope: Option<Slope>,
}
