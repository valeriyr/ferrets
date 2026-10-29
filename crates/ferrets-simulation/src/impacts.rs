//! Damage released by a weapon that has not landed yet.

use bevy_ecs::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use crate::{entity_def, simulation_id::SimulationId};
use ferrets_content::{
    attack::{AttackDef, Weapon},
    entity_type_def::{EntityTypeDef, EntityTypeId},
    projectile::ProjectileId,
    registry::ContentRegistry,
    turret::TurretId,
};

/// Which of an attacker's weapons a hit came from.
///
/// Carried rather than looked up, because a body may point one weapon and mount
/// several turrets, and because a shot in the air outlives the form that fired
/// it: a keep that morphs mid-flight does not re-aim what has already left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiredFrom {
    /// The weapon the body points itself.
    Body,
    /// One of the turrets it carries.
    Turret(TurretId),
}

impl FiredFrom {
    /// The weapon this names on `def`.
    pub fn weapon<'a>(
        self,
        registry: &'a ContentRegistry,
        def: &'a EntityTypeDef,
    ) -> Option<&'a Weapon> {
        match self {
            FiredFrom::Body => def.attack.as_ref().map(AttackDef::weapon),
            FiredFrom::Turret(turret) => Some(registry.turret_def(turret).weapon()),
        }
    }
}

/// What a weapon is worked against this tick.
#[derive(Debug, Clone, Copy)]
pub(crate) enum AimedAt {
    /// A body: measured, faced and hit by the cells it stands on.
    Body(Entity),
    /// A bare point on the map.
    Point(FixedUVec2),
}

impl AimedAt {
    /// The body aimed at, if one is.
    pub(crate) fn body(self) -> Option<Entity> {
        match self {
            AimedAt::Body(body) => Some(body),
            AimedAt::Point(_) => None,
        }
    }

    /// The cells an attack on it is measured to: every cell the body stands
    /// on, or the cell the point lies in.
    pub(crate) fn reached(self, world: &World) -> CellRect {
        match self {
            AimedAt::Body(body) => entity_def::standing_rect(world, body),
            AimedAt::Point(point) => CellRect::new(CellPos::from(point), CellSize::ONE),
        }
    }

    /// Where it lies and the room it takes, as a weapon comes round on it.
    pub(crate) fn footprint(self, world: &World) -> (FixedUVec2, CellSize) {
        match self {
            AimedAt::Body(body) => entity_def::footprint(world, body),
            AimedAt::Point(point) => (point, CellSize::ONE),
        }
    }
}

/// One shot in flight.
///
/// Holds ids and a content handle rather than [`Entity`] references, so a shot
/// outlives the entity that fired it: the firing type's damage bonuses stay
/// readable from the registry after the attacker is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingImpact {
    /// Who fired, recorded on the victim as the damage source.
    pub attacker: SimulationId,
    /// The firing type, for the damage bonuses that outlive the attacker.
    pub attacker_type: EntityTypeId,
    /// Which of that type's weapons fired it — a shot in the air is not re-aimed
    /// by a form change behind it.
    pub fired_from: FiredFrom,
    /// Which projectile kind is in the air, so a renderer can tell an arrow from a
    /// cannonball.
    pub projectile: ProjectileId,
    /// Whom the shot follows.
    pub target: Following,
    /// Where the shot was released, for measuring a line blast and for drawing.
    pub origin: FixedUVec2,
    /// Where the shot lands unless it follows a target: the cell it was aimed at,
    /// or where a target it lost went in; a following shot falls back on it once
    /// its target is gone.
    pub impact: FixedUVec2,
    /// The attacker's effective damage, frozen when the shot was released.
    pub damage: FixedU64,
    /// The tick the shot was released on.
    pub emitted_on_tick: u32,
    /// The tick the shot lands on.
    pub lands_on_tick: u32,
}

/// Whom a shot in flight follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Following {
    /// Nobody: a shot sent to a cell hits whoever stands there.
    Nothing,
    /// This entity, the shot's only direct victim, wherever it stands — or, in
    /// its dying phase, where it lies; gone, it leaves the shot no direct
    /// victim.
    Target(SimulationId),
    /// A target that went off the map while the shot flew: no direct victim,
    /// and the shot lands where it went in.
    Lost,
}

/// Every shot currently in flight, in release order.
///
/// Release order is deterministic because the order loop walks entities in a
/// fixed order, so every peer resolves the same shots in the same sequence.
#[derive(Resource, Debug, Default)]
pub struct PendingImpacts {
    in_flight: Vec<PendingImpact>,
}

impl PendingImpacts {
    /// Queues a shot.
    pub fn push(&mut self, impact: PendingImpact) {
        self.in_flight.push(impact);
    }

    /// Removes and returns every shot due on or before `tick`, in release order.
    pub fn take_due(&mut self, tick: u32) -> Vec<PendingImpact> {
        let (due, pending) = self
            .in_flight
            .iter()
            .partition(|impact| impact.lands_on_tick <= tick);
        self.in_flight = pending;
        due
    }

    /// Turns every shot following `target` into one that lost it, landing at
    /// `went_in`.
    pub fn lose(&mut self, target: SimulationId, went_in: FixedUVec2) {
        for shot in &mut self.in_flight {
            match shot.target {
                Following::Target(id) if id == target => {
                    shot.target = Following::Lost;
                    shot.impact = went_in;
                }
                Following::Target(_) | Following::Lost | Following::Nothing => {}
            }
        }
    }

    /// Every shot still in flight, in release order — for rendering.
    pub fn in_flight(&self) -> &[PendingImpact] {
        &self.in_flight
    }
}
