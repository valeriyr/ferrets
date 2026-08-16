//! The targeting-layer predicate: which weapons can reach which victims.
//!
//! Nothing here is stored. A weapon names the layers it reaches — a required
//! declaration on every armed type, because any default aims the weapon
//! somewhere the author never said — and a victim names the layers it can be
//! attacked on, defaulting to where it lives. The two are matched by
//! intersection: any layer in common is enough.
//!
//! Intersection, not containment: a victim answerable on two layers is
//! answerable by a weapon reaching either of them, which is what lets a thing
//! rooted on the ground still be shot out of the air.

use ferrets_pathfinder::layer_mask::LayerMask;

use crate::entity_type_def::EntityTypeDef;

/// The layers `def` can be attacked on: its own declaration, or the layers it
/// occupies.
///
/// A type with no location at all is answerable on no layer, which only arises
/// for content the registry would already have rejected.
pub fn targetable(def: &EntityTypeDef) -> LayerMask {
    def.targetable.unwrap_or_else(|| {
        def.location
            .map_or(LayerMask::EMPTY, |location| location.occupation())
    })
}

/// Whether `attacker`'s weapon can reach `victim`.
///
/// An attacker that names no layers reaches nothing. That only arises for a
/// type with no weapon — the registry requires every armed type to declare
/// its targets — and an unarmed type never gets past the capability checks
/// that guard every attack path, so the arm is a fail-closed residue.
pub fn reaches(attacker: &EntityTypeDef, victim: &EntityTypeDef) -> bool {
    match attacker.attack {
        None => false,
        Some(attack) => attack.targets() & targetable(victim) != LayerMask::EMPTY,
    }
}
