//! The field vocabulary: per-cell areas that standing entities project, the
//! sources that feed them, the placement rules that read them, and the effects
//! they have on whatever stands inside or outside.

use ferrets_nav_grid::{layer_mask::LayerMask, terrain_mask::TerrainMask};

use crate::{
    affiliation::Affiliation, area::Area, coverage::Coverage, detection::Detection,
    entity_effect::EntityEffect, requirement::Requirement, sight_block::SightBlock,
    terrains::Terrains,
};

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

/// A field's watch over the cells it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldWatch {
    /// Whom a covered cell is in sight of, judged from the players covering
    /// it.
    of: Affiliation,
}

impl FieldWatch {
    /// Creates a new `FieldWatch` with the given data.
    pub fn new(of: Affiliation) -> Self {
        Self { of }
    }

    /// Whom a covered cell is in sight of, judged from the players covering
    /// it.
    #[inline]
    pub fn of(&self) -> Affiliation {
        self.of
    }
}

/// One kind of field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDef {
    /// The terrains its cells may lie on. A cell of another terrain is never
    /// covered, a source's own footprint included, so a body standing partly
    /// on one is never wholly inside the field. Cover spreads from a
    /// footprint through such cells only: one cut off by other terrain stays
    /// uncovered, even within reach.
    terrains: Terrains,
    /// The layers it spreads on: whose bodies its effects and placement rules
    /// are for.
    spreads_on: LayerMask,
    /// What happens to covered cells no source sustains.
    decay: FieldDecay,
    /// Whom the cells it covers are in sight of, if anyone's.
    watches: Option<FieldWatch>,
    /// What each cell it covers blocks of the sight across it, if anything.
    blocks_sight: Option<SightBlock>,
    /// What covering a cell does for the players covering it against the
    /// concealed entities standing there.
    detection: Detection,
}

impl FieldDef {
    /// Creates a new `FieldDef` with the given data.
    ///
    /// Panics if the field lies on no terrain at all, spreads on no layer, if a
    /// gradual decay has a zero cycle, or if the detection reveals no layer at
    /// all.
    pub fn new(
        terrains: Terrains,
        spreads_on: LayerMask,
        decay: FieldDecay,
        watches: Option<FieldWatch>,
        blocks_sight: Option<SightBlock>,
        detection: Detection,
    ) -> Self {
        match terrains {
            Terrains::Only(terrains) => assert!(
                terrains != TerrainMask::EMPTY,
                "a field on no terrain lies nowhere; declare it on \"any\""
            ),
            Terrains::Any => {}
        }
        assert!(
            spreads_on != LayerMask::EMPTY,
            "a field spreading on no layer is for nobody; declare the layers it spreads on"
        );
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
            terrains,
            spreads_on,
            decay,
            watches,
            blocks_sight,
            detection,
        }
    }

    /// The terrains its cells may lie on.
    #[inline]
    pub fn terrains(&self) -> Terrains {
        self.terrains
    }

    /// The layers it spreads on: whose bodies its effects and placement rules
    /// are for.
    #[inline]
    pub fn spreads_on(&self) -> LayerMask {
        self.spreads_on
    }

    /// What happens to covered cells no source sustains.
    #[inline]
    pub fn decay(&self) -> FieldDecay {
        self.decay
    }

    /// Whom the cells it covers are in sight of, if anyone's.
    #[inline]
    pub fn watches(&self) -> Option<FieldWatch> {
        self.watches
    }

    /// What each cell it covers blocks of the sight across it, if anything.
    #[inline]
    pub fn blocks_sight(&self) -> Option<SightBlock> {
        self.blocks_sight
    }

    /// What covering a cell does for the players covering it against the
    /// concealed entities standing there.
    #[inline]
    pub fn detection(&self) -> Detection {
        self.detection
    }
}

/// How a source's reach comes to cover its area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldGrowth {
    /// The whole area is covered the tick the source stands.
    Instant,
    /// The reach starts at `starts_at` and grows by one cell every
    /// `cycle` ticks until it spans the area.
    Gradual {
        /// Ticks between growth steps.
        cycle: u32,
        /// The reach growth starts at, no farther than the area, read at the
        /// source's first recompute once its body has risen, or while rising
        /// if it projects in full then.
        starts_at: Area,
    },
}

/// What an act does to a field around its performer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldAction {
    /// Covers the cells within the area on the field's terrains.
    Cover,
    /// Clears the cells within the area that no source sustains.
    Clear,
}

/// What a source projects while the entity it belongs to is not operating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// As much as when the entity operates.
    Full,
    /// A patch of this area around the footprint, no farther than the
    /// source's area, read on every recompute.
    Held(Area),
    /// Nothing; what it covered recedes by the field's decay.
    Nothing,
}

/// One field an entity type projects, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSourceDef {
    /// The field projected.
    field: FieldId,
    /// The area around the footprint the field reaches, read on every
    /// recompute.
    area: Area,
    /// How the reach comes to cover the area.
    growth: FieldGrowth,
    /// What it projects while the entity is still rising.
    while_rising: Emission,
    /// What it projects while the entity is disabled.
    while_disabled: Emission,
}

impl FieldSourceDef {
    /// Creates a new `FieldSourceDef` with the given data.
    ///
    /// Panics if a gradual growth has a zero cycle.
    pub fn new(
        field: FieldId,
        area: Area,
        growth: FieldGrowth,
        while_rising: Emission,
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
            area,
            growth,
            while_rising,
            while_disabled,
        }
    }

    /// The field projected.
    #[inline]
    pub fn field(&self) -> FieldId {
        self.field
    }

    /// The area around the footprint the field reaches, read on every
    /// recompute.
    #[inline]
    pub fn area(&self) -> Area {
        self.area
    }

    /// How the reach comes to cover the area.
    #[inline]
    pub fn growth(&self) -> FieldGrowth {
        self.growth
    }

    /// What it projects while the entity is rising.
    #[inline]
    pub fn while_rising(&self) -> Emission {
        self.while_rising
    }

    /// What it projects while the entity is disabled.
    #[inline]
    pub fn while_disabled(&self) -> Emission {
        self.while_disabled
    }
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
        coverage: Coverage,
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

/// Which side of a field an entity stands on, for an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSide {
    /// As much of the entity's footprint is covered as the effect's coverage
    /// asks for.
    Inside,
    /// Not inside.
    Outside,
}

/// What a field does to an entity type on each side of it: inside when as
/// much of its footprint is covered as the coverage asks for, outside
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEffect {
    /// The field read.
    field: FieldId,
    /// Whose coverage counts.
    of: Affiliation,
    /// How much of the bearer's footprint must be covered for it to stand
    /// inside; a footprint cell on terrain the field rejects is never
    /// covered.
    coverage: Coverage,
    /// What the field does to the bearer inside it.
    inside: Vec<EntityEffect>,
    /// What the field does to the bearer outside it.
    outside: Vec<EntityEffect>,
    /// What the bearer must meet besides the field for the effect to hold;
    /// `None` when the field alone decides.
    holds_while: Option<Requirement>,
}

impl FieldEffect {
    /// Creates a new `FieldEffect` with the given data.
    pub fn new(
        field: FieldId,
        of: Affiliation,
        coverage: Coverage,
        inside: Vec<EntityEffect>,
        outside: Vec<EntityEffect>,
        holds_while: Option<Requirement>,
    ) -> Self {
        Self {
            field,
            of,
            coverage,
            inside,
            outside,
            holds_while,
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

    /// How much of the bearer's footprint must be covered for it to stand
    /// inside.
    #[inline]
    pub fn coverage(&self) -> Coverage {
        self.coverage
    }

    /// What the field does to the bearer on `side`.
    #[inline]
    pub fn on(&self, side: FieldSide) -> &[EntityEffect] {
        match side {
            FieldSide::Inside => &self.inside,
            FieldSide::Outside => &self.outside,
        }
    }

    /// Every effect, inside then outside.
    #[inline]
    pub fn effects(&self) -> impl Iterator<Item = &EntityEffect> {
        self.inside.iter().chain(&self.outside)
    }

    /// What the bearer must meet besides the field for the effect to hold;
    /// `None` when the field alone decides.
    #[inline]
    pub fn holds_while(&self) -> Option<&Requirement> {
        self.holds_while.as_ref()
    }
}
