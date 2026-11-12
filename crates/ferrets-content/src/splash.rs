//! Content-defined splash property struct and its blast shape.

use ferrets_math::FixedU64;
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::{area::Area, quantity::Quantity};

/// How a blast's bands are measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplashShape {
    /// Bands measure distance from the impact point.
    Circular,
    /// Bands measure distance from the line the shot travelled, so the blast
    /// sweeps everything along its path rather than pooling at the end.
    Line,
}

/// Content-defined spread of a weapon's damage over an area.
///
/// A type without this definition damages only the entity it hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplashDef {
    /// How the bands are measured.
    shape: SplashShape,
    /// The area the innermost band takes in, and the fraction of full
    /// strength its hits land at. The area is read when the shot is released
    /// and held while it flies.
    core: (Area, FixedU64),
    /// The bands around the core, outward: how many cells further each
    /// reaches than the band inside it, and the fraction of full strength its
    /// hits land at. A victim takes the fraction of the innermost band it
    /// falls inside. The widths are read when the shot is released and held
    /// while it flies.
    rings: Vec<(Quantity, FixedU64)>,
    /// The navigation layers the blast reaches: a victim is caught only when the
    /// layers it can be attacked on intersect this mask, so content decides
    /// whether a ground blast touches what flies over it.
    layers: LayerMask,
    /// Whether the blast also damages the attacker's own and allied entities.
    friendly_fire: bool,
}

impl SplashDef {
    /// Creates a new `SplashDef` with the given data.
    ///
    /// Panics if a ring of a constant width `0` reaches no further than the
    /// band inside it (any but the first ring around an `Inside` core), or
    /// `layers` is empty (a blast that reaches no layer can never hit
    /// anything).
    pub fn new(
        shape: SplashShape,
        core: (Area, FixedU64),
        rings: Vec<(Quantity, FixedU64)>,
        layers: impl Into<LayerMask>,
        friendly_fire: bool,
    ) -> Self {
        // Around an `Inside` core, a first ring of width 0 still reaches the
        // bodies touching the one hit.
        let exempt = match core.0 {
            Area::Inside => 1,
            Area::Around(_) => 0,
        };
        for &(width, _) in rings.iter().skip(exempt) {
            assert!(
                width != Quantity::from(0),
                "a splash ring of constant width 0 reaches no further than the band inside it"
            );
        }
        let layers = layers.into();
        assert!(
            layers != LayerMask::EMPTY,
            "splash layers must not be empty"
        );

        Self {
            shape,
            core,
            rings,
            layers,
            friendly_fire,
        }
    }

    /// Returns how the bands are measured.
    #[inline]
    pub fn shape(&self) -> SplashShape {
        self.shape
    }

    /// Returns the layers the blast reaches.
    #[inline]
    pub fn layers(&self) -> LayerMask {
        self.layers
    }

    /// Returns whether the blast damages the attacker's own and allied entities.
    #[inline]
    pub fn friendly_fire(&self) -> bool {
        self.friendly_fire
    }

    /// The area the innermost band takes in, and the fraction of full
    /// strength its hits land at.
    #[inline]
    pub fn core(&self) -> (Area, FixedU64) {
        self.core
    }

    /// The bands around the core, outward: `(width beyond the band inside,
    /// fraction of full strength)`.
    pub fn rings(&self) -> impl Iterator<Item = (Quantity, FixedU64)> + '_ {
        self.rings.iter().copied()
    }
}
