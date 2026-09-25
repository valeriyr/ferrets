//! The field vocabulary: per-cell areas that standing entities project, the
//! sources that feed them, the placement rules that read them, and the effects
//! they have on whatever stands inside or outside.

use ferrets_pathfinder::layer_mask::LayerMask;

use crate::{affiliation::Affiliation, detection::Detection, entity_effect::EntityEffect};

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

/// Where a field may lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldLayer {
    /// Any cell of the map.
    Anywhere,
    /// The cells passable on every one of these layers.
    Passable(LayerMask),
}

/// One kind of field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDef {
    /// Where its cells may lie.
    layer: FieldLayer,
    /// What happens to covered cells no source sustains.
    decay: FieldDecay,
    /// What covering a cell does for the sight of the players covering it.
    vision: FieldVision,
    /// What covering a cell does for the players covering it against the
    /// concealed entities standing there.
    detection: Detection,
}

impl FieldDef {
    /// Creates a new `FieldDef` with the given data.
    ///
    /// Panics if a gradual decay has a zero cycle, if the field lies on cells
    /// passable on no layer at all, or if the detection reveals no layer at all.
    pub fn new(
        layer: FieldLayer,
        decay: FieldDecay,
        vision: FieldVision,
        detection: Detection,
    ) -> Self {
        match layer {
            FieldLayer::Passable(layers) => assert!(
                layers != LayerMask::EMPTY,
                "a field passable on no layer lies anywhere; declare it so"
            ),
            FieldLayer::Anywhere => {}
        }
        match decay {
            FieldDecay::Gradual { cycle } => {
                assert!(cycle > 0, "decay cycle must be positive");
            }
            FieldDecay::Instant | FieldDecay::Never => {}
        }
        match detection {
            Detection::Reveals(layers) => assert!(
                layers != LayerMask::EMPTY,
                "a field revealing no layer detects nothing; declare it blind"
            ),
            Detection::Blind => {}
        }
        Self {
            layer,
            decay,
            vision,
            detection,
        }
    }

    /// Where its cells may lie.
    #[inline]
    pub fn layer(&self) -> FieldLayer {
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

    /// What covering a cell does for the players covering it against the
    /// concealed entities standing there.
    #[inline]
    pub fn detection(&self) -> Detection {
        self.detection
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

/// What a source projects while the entity it belongs to is not operating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// As much as when the entity operates.
    Full,
    /// A patch this many cells out from the footprint, no farther than the
    /// source's radius.
    Held(u32),
    /// Nothing; what it covered recedes by the field's decay.
    Nothing,
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
    /// What it projects while the entity is still under construction.
    while_constructing: Emission,
    /// What it projects while the entity is disabled.
    while_disabled: Emission,
}

impl FieldSourceDef {
    /// Creates a new `FieldSourceDef` with the given data.
    ///
    /// Panics if a gradual growth has a zero cycle.
    pub fn new(
        field: FieldId,
        radius: u32,
        growth: FieldGrowth,
        while_constructing: Emission,
        while_disabled: Emission,
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
            while_disabled,
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

    /// What it projects while the entity is under construction.
    #[inline]
    pub fn while_constructing(&self) -> Emission {
        self.while_constructing
    }

    /// What it projects while the entity is disabled.
    #[inline]
    pub fn while_disabled(&self) -> Emission {
        self.while_disabled
    }
}

/// How much of a footprint must answer a field's question for the answer to
/// stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCoverage {
    /// Every cell of the footprint.
    Every,
    /// Any one cell of it.
    Any,
}

/// One rule a field imposes on where an entity type may be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPlacement {
    /// The read cells must all be covered by the field.
    Requires {
        /// The field read.
        field: FieldId,
        /// Whose coverage counts.
        of: Affiliation,
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
    /// Enough of the entity's footprint is covered, as its coverage asks.
    Inside,
    /// Enough of the entity's footprint is uncovered, as its coverage asks.
    Outside,
}

/// One effect a field has on an entity type, while enough of its footprint is
/// on the given side of the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEffect {
    /// The field read.
    field: FieldId,
    /// Whose coverage counts.
    of: Affiliation,
    /// The side of the field the effect applies on.
    side: FieldSide,
    /// How much of the bearer's footprint must be on that side.
    coverage: FieldCoverage,
    /// What the effect does.
    kind: EntityEffect,
}

impl FieldEffect {
    /// Creates a new `FieldEffect` with the given data.
    pub fn new(
        field: FieldId,
        of: Affiliation,
        side: FieldSide,
        coverage: FieldCoverage,
        kind: EntityEffect,
    ) -> Self {
        Self {
            field,
            of,
            side,
            coverage,
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
    pub fn of(&self) -> Affiliation {
        self.of
    }

    /// The side of the field the effect applies on.
    #[inline]
    pub fn side(&self) -> FieldSide {
        self.side
    }

    /// How much of the bearer's footprint must be on that side.
    #[inline]
    pub fn coverage(&self) -> FieldCoverage {
        self.coverage
    }

    /// What the effect does.
    #[inline]
    pub fn kind(&self) -> &EntityEffect {
        &self.kind
    }
}
