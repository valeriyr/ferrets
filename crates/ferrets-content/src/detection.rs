//! What a cover reveals of the concealed entities on the cells it holds.

use ferrets_pathfinder::layer_mask::LayerMask;

/// What a cover — a field, a watch — does for the concealed entities standing
/// on the cells it holds, toward the side holding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    /// Nothing: a concealed entity under it stays as it was.
    Blind,
    /// Those standing on any of the named layers are seen.
    Reveals(LayerMask),
}
