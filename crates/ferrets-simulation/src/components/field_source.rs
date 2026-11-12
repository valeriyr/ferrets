//! Per-source reach state for the fields an entity projects.

use bevy_ecs::prelude::*;
use ferrets_content::field::{FieldGrowth, FieldId, FieldSourceDef};

/// How far an entity's field sources reach the moment it enters the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldReach {
    /// Each source starts at its declared growth start and grows from there.
    Initial,
    /// Each source already spans its whole area.
    Full,
}

/// How far a source's field reaches around its footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Growth {
    /// Not read yet: the source starts where its arrival asked, its start
    /// read at its first recompute once its body has risen, or while rising
    /// if it projects in full then.
    Unread(FieldReach),
    /// Read, for a source that spans its whole area at once.
    Spanning {
        /// Cells out from the footprint.
        reach: u32,
    },
    /// Read, for a source that grows a cell every cycle.
    Growing {
        /// Cells out from the footprint.
        reach: u32,
        /// Ticks until the reach grows by one cell.
        countdown: u32,
    },
}

impl Growth {
    /// How far the field reaches around the footprint, once read.
    pub fn reach(self) -> Option<u32> {
        match self {
            Growth::Unread(_) => None,
            Growth::Spanning { reach } | Growth::Growing { reach, .. } => Some(reach),
        }
    }

    /// This read growth reaching `reach` instead, its countdown kept.
    ///
    /// Panics for an unread start, which has no reach to replace.
    pub fn with_reach(self, reach: u32) -> Growth {
        match self {
            Growth::Unread(_) => panic!("only a read growth has its reach replaced"),
            Growth::Spanning { .. } => Growth::Spanning { reach },
            Growth::Growing { countdown, .. } => Growth::Growing { reach, countdown },
        }
    }
}

/// What a source put on the grid at the last recompute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emitted {
    /// Nothing: not recomputed yet, or emitting nothing while rising or
    /// disabled.
    Nothing,
    /// A patch reaching this many cells from the footprint.
    Reach(u32),
}

/// The live state of one declared field source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSourceState {
    /// The field the source projects.
    pub field: FieldId,
    /// How far the source's field reaches around its footprint.
    pub growth: Growth,
    /// What it put on the grid at the last recompute.
    pub last_emitted: Emitted,
}

impl FieldSourceState {
    /// The state a source of `def` starts in, reaching as `start` asks.
    pub fn new(def: &FieldSourceDef, start: FieldReach) -> Self {
        Self {
            field: def.field(),
            growth: Growth::Unread(start),
            last_emitted: Emitted::Nothing,
        }
    }

    /// The state a source of `def` takes over from `previous`: an instant
    /// source spans its whole area, a gradual one carries the growth on and
    /// starts a fresh growth cycle; either keeps what `previous` put on the
    /// grid.
    pub fn taking_over(def: &FieldSourceDef, previous: &FieldSourceState) -> Self {
        let growth = match def.growth() {
            FieldGrowth::Instant => Growth::Unread(FieldReach::Full),
            FieldGrowth::Gradual { cycle, .. } => match previous.growth {
                Growth::Unread(start) => Growth::Unread(start),
                Growth::Spanning { reach } | Growth::Growing { reach, .. } => Growth::Growing {
                    reach,
                    countdown: cycle,
                },
            },
        };
        Self {
            field: def.field(),
            growth,
            last_emitted: previous.last_emitted,
        }
    }

    /// How far the source's field reaches around its footprint: its read
    /// reach, or, not read yet, the patch it put on the grid; `None` for an
    /// unread source that put nothing there.
    pub fn reach(self) -> Option<u32> {
        match (self.growth.reach(), self.last_emitted) {
            (Some(reach), _) => Some(reach),
            (None, Emitted::Reach(patch)) => Some(patch),
            (None, Emitted::Nothing) => None,
        }
    }
}

/// The live state of every field source an entity's type declares, in
/// declaration order.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct FieldSourcesComponent(pub Vec<FieldSourceState>);

impl FieldSourcesComponent {
    /// One state per declared source, each reaching as `start` asks.
    pub fn new(defs: &[FieldSourceDef], start: FieldReach) -> Self {
        Self(
            defs.iter()
                .map(|def| FieldSourceState::new(def, start))
                .collect(),
        )
    }

    /// Starts every source over at its growth start, as a fresh arrival's,
    /// emitting nothing until it is read again.
    pub fn start_over(&mut self) {
        for state in &mut self.0 {
            state.growth = Growth::Unread(FieldReach::Initial);
            state.last_emitted = Emitted::Nothing;
        }
    }

    /// One state per declared source, each taking over from the first
    /// source of the same field in `previous` not yet taken over; a source
    /// with no such predecessor starts at its growth start. Sources of one
    /// field match in their declared order, so two of them listed the other
    /// way around in the new form swap what they take over.
    pub fn taking_over(defs: &[FieldSourceDef], previous: &Self) -> Self {
        let mut taken = vec![false; previous.0.len()];
        Self(
            defs.iter()
                .map(|def| {
                    let predecessor = previous
                        .0
                        .iter()
                        .enumerate()
                        .find(|&(index, state)| !taken[index] && state.field == def.field());
                    match predecessor {
                        Some((index, state)) => {
                            taken[index] = true;
                            FieldSourceState::taking_over(def, state)
                        }
                        None => FieldSourceState::new(def, FieldReach::Initial),
                    }
                })
                .collect(),
        )
    }
}
