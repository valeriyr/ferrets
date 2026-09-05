//! The field vocabulary: per-cell areas that standing entities project, the
//! sources that feed them, the placement rules that read them, and the effects
//! they have on whatever stands inside or outside.

use ferrets_pathfinder::layer_mask::LayerMask;

use crate::stats::EntityModifier;

/// A handle to a registered field kind, assigned in registration order.
///
/// Content declares fields by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldId(u16);

impl FieldId {
    /// Creates a field id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more fields registered than FieldId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What happens to a covered cell once no source sustains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldDecay {
    /// The cell clears the tick its last source stops.
    Instant,
    /// Cells clear from the edge inward, one ring every `cycle` ticks.
    Gradual {
        /// Ticks between recession steps.
        cycle: u32,
    },
    /// The cell stays covered until something clears it.
    Never,
}

/// What covering a cell does for the sight of the players covering it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldVision {
    /// Nothing: a covered cell is as dark as any other.
    Dark,
    /// Every covered cell is in sight of the players covering it.
    Watched,
}

/// One kind of field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDef {
    /// The layers a cell's terrain must pass for the field to cover it. An
    /// empty mask covers any cell.
    layer: LayerMask,
    /// What happens to covered cells no source sustains.
    decay: FieldDecay,
    /// What covering a cell does for the sight of the players covering it.
    vision: FieldVision,
}

impl FieldDef {
    /// Creates a new `FieldDef` with the given data.
    ///
    /// Panics if a gradual decay has a zero cycle.
    pub fn new(layer: impl Into<LayerMask>, decay: FieldDecay, vision: FieldVision) -> Self {
        match decay {
            FieldDecay::Gradual { cycle } => {
                assert!(cycle > 0, "decay cycle must be positive");
            }
            FieldDecay::Instant | FieldDecay::Never => {}
        }
        Self {
            layer: layer.into(),
            decay,
            vision,
        }
    }

    /// The layers a cell's terrain must pass for the field to cover it.
    #[inline]
    pub fn layer(&self) -> LayerMask {
        self.layer
    }

    /// What happens to covered cells no source sustains.
    #[inline]
    pub fn decay(&self) -> FieldDecay {
        self.decay
    }

    /// What covering a cell does for the sight of the players covering it.
    #[inline]
    pub fn vision(&self) -> FieldVision {
        self.vision
    }
}

/// How a source's reach comes to cover its radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldGrowth {
    /// The whole radius is covered the tick the source stands.
    Instant,
    /// The reach starts at `initial_radius` and grows by one cell every
    /// `cycle` ticks until it spans the radius.
    Gradual {
        /// Ticks between growth steps.
        cycle: u32,
        /// The reach a source starts with.
        initial_radius: u32,
    },
}

/// What an act does to a field around its performer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldAction {
    /// Covers the cells within radius that pass the field's layer.
    Cover,
    /// Clears the cells within radius that no source sustains.
    Clear,
}

/// One field an entity type projects, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSourceDef {
    /// The field projected.
    field: FieldId,
    /// How far from the footprint the field reaches, in cells. Zero reaches
    /// the footprint cells alone.
    radius: u32,
    /// How the reach comes to cover the radius.
    growth: FieldGrowth,
    /// A radius projected while the source is still under construction.
    /// `None` projects nothing until it stands.
    while_constructing: Option<u32>,
}

impl FieldSourceDef {
    /// Creates a new `FieldSourceDef` with the given data.
    ///
    /// Panics if a gradual growth has a zero cycle.
    pub fn new(
        field: FieldId,
        radius: u32,
        growth: FieldGrowth,
        while_constructing: Option<u32>,
    ) -> Self {
        match growth {
            FieldGrowth::Gradual { cycle, .. } => {
                assert!(cycle > 0, "growth cycle must be positive");
            }
            FieldGrowth::Instant => {}
        }
        Self {
            field,
            radius,
            growth,
            while_constructing,
        }
    }

    /// The field projected.
    #[inline]
    pub fn field(&self) -> FieldId {
        self.field
    }

    /// How far from the footprint the field reaches, in cells.
    #[inline]
    pub fn radius(&self) -> u32 {
        self.radius
    }

    /// How the reach comes to cover the radius.
    #[inline]
    pub fn growth(&self) -> FieldGrowth {
        self.growth
    }

    /// The radius projected while under construction, if any.
    #[inline]
    pub fn while_constructing(&self) -> Option<u32> {
        self.while_constructing
    }
}

/// Whose coverage of a cell counts, judged from the entity's owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldAffiliation {
    /// Only the owner's own coverage.
    Own,
    /// Coverage by the owner or any of its allies.
    Allied,
    /// Coverage by anyone.
    Anyone,
}

/// Which cells of a footprint a placement rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCoverage {
    /// Every cell of the footprint.
    Footprint,
    /// The anchor cell alone.
    Anchor,
}

/// One rule a field imposes on where an entity type may be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPlacement {
    /// The read cells must all be covered by the field.
    Requires {
        /// The field read.
        field: FieldId,
        /// Whose coverage counts.
        of: FieldAffiliation,
        /// Which footprint cells are read.
        coverage: FieldCoverage,
    },
    /// No footprint cell may be covered by the field, by anyone.
    Forbids {
        /// The field read.
        field: FieldId,
    },
}

impl FieldPlacement {
    /// The field the rule reads.
    #[inline]
    pub fn field(&self) -> FieldId {
        match *self {
            FieldPlacement::Requires { field, .. } | FieldPlacement::Forbids { field } => field,
        }
    }
}

/// Which side of a field an effect applies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSide {
    /// The entity's anchor cell is covered.
    Inside,
    /// The entity's anchor cell is not covered.
    Outside,
}

/// What a field effect does to the entity while it applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldEffectKind {
    /// Folds the modifiers into the entity's effective stats.
    Modifiers(Vec<EntityModifier>),
    /// The entity stands but does not operate: it starts no order but Train
    /// and Research, which wait, and neither fights, hunts, casts nor moves.
    Disabled,
}

/// One effect a field has on an entity type, while its anchor cell is on the
/// given side of the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEffect {
    /// The field read.
    field: FieldId,
    /// Whose coverage counts.
    of: FieldAffiliation,
    /// The side of the field the effect applies on.
    side: FieldSide,
    /// What the effect does.
    kind: FieldEffectKind,
}

impl FieldEffect {
    /// Creates a new `FieldEffect` with the given data.
    pub fn new(
        field: FieldId,
        of: FieldAffiliation,
        side: FieldSide,
        kind: FieldEffectKind,
    ) -> Self {
        Self {
            field,
            of,
            side,
            kind,
        }
    }

    /// The field read.
    #[inline]
    pub fn field(&self) -> FieldId {
        self.field
    }

    /// Whose coverage counts.
    #[inline]
    pub fn of(&self) -> FieldAffiliation {
        self.of
    }

    /// The side of the field the effect applies on.
    #[inline]
    pub fn side(&self) -> FieldSide {
        self.side
    }

    /// What the effect does.
    #[inline]
    pub fn kind(&self) -> &FieldEffectKind {
        &self.kind
    }
}
