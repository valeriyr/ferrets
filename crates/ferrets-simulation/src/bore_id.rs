//! Deterministic tunnel bore identifier, stable across all peers and replays.

use bevy_ecs::resource::Resource;

/// One tunnel's bore: the inside a pair of ends opens, kept while either end
/// stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoreId(pub u32);

/// Mints each new bore's id, in the same order on every peer.
#[derive(Resource, Debug, Default)]
pub struct BoreIds {
    /// The id minted last.
    last: u32,
}

impl BoreIds {
    /// Mints the next bore's id.
    pub fn mint(&mut self) -> BoreId {
        self.last += 1;
        BoreId(self.last)
    }
}
