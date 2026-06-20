//! Death-phase configuration and markers for simulation entities.

use bevy_ecs::prelude::*;

/// Content-defined configuration of an entity's dying phase: how long it lasts
/// and what, if anything, it leaves behind.
///
/// A destroyed entity spends `dying_time` ticks in this phase before it is
/// removed. Destruction is independent of health — depleted resource sources,
/// cancelled constructions, and scripted removals all destroy entities that may
/// never take damage.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct DyingStaticData {
    /// Ticks the dying phase lasts.
    dying_time: u32,
    /// The entity type left behind when the dying phase completes. `None`
    /// means the entity disappears without a trace.
    corpse_type: Option<String>,
}

impl DyingStaticData {
    /// Creates a new `DyingStaticData` with the given data. `corpse_type` is
    /// the entity left behind when the dying phase completes; it decays through
    /// its own dying phase, so chained corpse types form decay stages
    /// (corpse → bones → gone).
    ///
    /// Panics if `dying_time` is `0` (an entity without a dying phase simply
    /// omits this component) or `corpse_type` is empty.
    pub fn new(dying_time: u32, corpse_type: Option<&str>) -> Self {
        assert!(dying_time > 0, "dying_time must be greater than 0");
        assert!(
            corpse_type.is_none_or(|corpse| !corpse.is_empty()),
            "corpse_type must not be empty"
        );

        Self {
            dying_time,
            corpse_type: corpse_type.map(String::from),
        }
    }

    /// Returns the duration of the dying phase in ticks.
    pub fn dying_time(&self) -> u32 {
        self.dying_time
    }

    /// Returns the entity type left behind when the dying phase completes.
    pub fn corpse_type(&self) -> Option<&str> {
        self.corpse_type.as_deref()
    }
}

/// Marks an entity that is in the process of dying.
///
/// A dying entity cannot be selected and does not accept new orders, but it
/// still holds its footprint on the navigation grid.
#[derive(Component, Debug)]
pub struct DyingComponent {
    /// Ticks left until the entity finishes dying and is removed from the world.
    pub ticks_remaining: u32,
}

/// Marks remains spawned directly into the dying state (a corpse, rubble).
///
/// Distinguishes the two meanings of a running [`DyingComponent`]: without this
/// marker the entity is transitioning out of the alive world (a death
/// animation); with it, the entity is remains whose dying timer is its entire
/// existence (decay).
#[derive(Component, Debug, Default)]
pub struct CorpseComponent;

/// Marks an entity whose dying phase has completed and is ready to be despawned.
#[derive(Component, Debug, Default)]
pub struct DiedComponent;
