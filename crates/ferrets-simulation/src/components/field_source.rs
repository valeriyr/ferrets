//! Per-source growth state for the fields an entity projects.

use bevy_ecs::prelude::*;
use ferrets_content::field::{FieldGrowth, FieldSourceDef};

/// The live state of one declared field source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSourceState {
    /// How far from the footprint the source currently reaches, in cells.
    pub reach: u32,
    /// Ticks until the reach grows by one cell.
    pub countdown: u32,
}

impl FieldSourceState {
    /// The state a source of `def` starts in.
    pub fn seeded(def: &FieldSourceDef) -> Self {
        let (reach, countdown) = match def.growth() {
            FieldGrowth::Instant => (def.radius(), 0),
            FieldGrowth::Gradual {
                cycle,
                initial_radius,
            } => (initial_radius, cycle),
        };
        Self { reach, countdown }
    }

    /// The state a source of `def` has once it spans its whole radius.
    pub fn full(def: &FieldSourceDef) -> Self {
        Self {
            reach: def.radius(),
            ..Self::seeded(def)
        }
    }

    /// The state a source of `def` takes over from one that reached `reach`:
    /// an instant source spans its radius, a gradual one carries the reach on,
    /// no further than its own radius, and starts a fresh growth cycle.
    pub fn carried(def: &FieldSourceDef, reach: u32) -> Self {
        match def.growth() {
            FieldGrowth::Instant => Self::full(def),
            FieldGrowth::Gradual { .. } => Self {
                reach: reach.min(def.radius()),
                ..Self::seeded(def)
            },
        }
    }
}

/// The live state of every field source an entity's type declares, in
/// declaration order.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct FieldSourcesComponent(pub Vec<FieldSourceState>);

impl FieldSourcesComponent {
    /// One state per declared source, each carrying on from the reach of the
    /// source at the same index in `previous`; a source with no predecessor is
    /// freshly seeded.
    pub fn carried(defs: &[FieldSourceDef], previous: &Self) -> Self {
        Self(
            defs.iter()
                .enumerate()
                .map(|(index, def)| match previous.0.get(index) {
                    Some(state) => FieldSourceState::carried(def, state.reach),
                    None => FieldSourceState::seeded(def),
                })
                .collect(),
        )
    }

    /// One freshly seeded state per declared source.
    pub fn seeded(defs: &[FieldSourceDef]) -> Self {
        Self(defs.iter().map(FieldSourceState::seeded).collect())
    }

    /// One state per declared source, each already spanning its radius.
    pub fn full(defs: &[FieldSourceDef]) -> Self {
        Self(defs.iter().map(FieldSourceState::full).collect())
    }
}
