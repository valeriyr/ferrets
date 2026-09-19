//! Death-phase markers for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_content::{attack::Slain, dying::DeathKind, entity_type_def::EntityTypeId};

use crate::session::player_id::PlayerId;

/// What an entity is going through, and so what it hands on at the end of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passing {
    /// A death: the kind its type's bequests are declared against, and what the
    /// weapon that landed the blow leaves of a body. Only a weapon can deny
    /// one, so a death nothing swung at leaves what its type declares.
    Death {
        /// The death it is dying of, as content names it.
        kind: DeathKind,
        /// What the weapon behind it leaves of a body.
        slain: Slain,
    },
    /// A removal nothing authored, which hands on nothing.
    Removal,
}

/// Marks an entity that is in the process of dying.
///
/// A dying entity cannot be selected and does not accept new orders, but it
/// still holds its footprint on the navigation grid.
#[derive(Component, Debug)]
pub struct DyingComponent {
    /// Ticks left until the entity finishes dying and is removed from the world.
    pub ticks_remaining: u32,
    /// What it is going through, and so what it hands on when the phase ends.
    pub passing: Passing,
}

/// Marks an entity left somewhere as remains, and remembers whose body it is.
///
/// Distinguishes the two meanings of a running [`DyingComponent`]: without this
/// marker the entity is transitioning out of the alive world (a death
/// animation); with it, the entity is remains whose dying timer is its entire
/// existence (decay).
#[derive(Component, Debug)]
pub struct RemainsComponent {
    /// The type that fell here.
    pub of: EntityTypeId,
    /// Who owned it — absent for remains left by something nobody owned.
    pub owner: Option<PlayerId>,
}

/// Marks an entity whose dying phase has completed and is ready to be despawned.
#[derive(Component, Debug, Default)]
pub struct DiedComponent;
