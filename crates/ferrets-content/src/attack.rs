//! Content-defined auto-attack capability: the non-scalar properties of a
//! type's weapon.
//!
//! The weapon itself is scalar stats — damage, ranges, cadence — living in
//! the type's base stats so the modifier pipeline can move them. What groups
//! here is what a weapon declares that no modifier can touch.

use ferrets_pathfinder::layer_mask::LayerMask;

/// Non-scalar properties of an entity type's weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackDef {
    /// The navigation layers the weapon can reach — see
    /// [`targeting::reaches`](crate::targeting::reaches).
    targets: LayerMask,
}

impl AttackDef {
    /// Creates a new `AttackDef` with the given data.
    ///
    /// Panics if `targets` is empty, which would leave the weapon unable to
    /// hit anything at all.
    pub fn new(targets: impl Into<LayerMask>) -> Self {
        let targets = targets.into();
        assert!(
            targets != LayerMask::EMPTY,
            "a weapon's targets must not be empty"
        );
        Self { targets }
    }

    /// The navigation layers the weapon can reach.
    #[inline]
    pub fn targets(&self) -> LayerMask {
        self.targets
    }
}
