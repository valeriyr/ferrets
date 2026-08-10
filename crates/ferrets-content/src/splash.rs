//! Content-defined splash property struct and its blast shape.

use ferrets_math::FixedU64;
use ferrets_pathfinder::layer_mask::LayerMask;

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
    /// `(radius in cells, fraction of the hit's damage)`, in increasing radius
    /// order. A victim takes the fraction of the first band it falls inside.
    bands: Vec<(u32, FixedU64)>,
    /// The navigation layers the blast reaches: a victim is caught only when its
    /// occupation intersects this mask, so content decides whether a ground blast
    /// touches what flies over it.
    layers: LayerMask,
    /// Whether the blast also damages the attacker's own and allied entities.
    friendly_fire: bool,
}

impl SplashDef {
    /// Creates a new `SplashDef` with the given data.
    ///
    /// Panics if `bands` is empty, its radii are not strictly increasing, or
    /// `layers` is empty (a blast that reaches no layer can never hit anything).
    pub fn new(
        shape: SplashShape,
        bands: Vec<(u32, FixedU64)>,
        layers: impl Into<LayerMask>,
        friendly_fire: bool,
    ) -> Self {
        assert!(!bands.is_empty(), "splash needs at least one band");
        assert!(
            bands.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "splash band radii must be strictly increasing"
        );

        let layers = layers.into();
        assert!(
            layers != LayerMask::EMPTY,
            "splash layers must not be empty"
        );

        Self {
            shape,
            bands,
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

    /// The outermost radius the blast reaches, for gathering candidate victims.
    pub fn reach(&self) -> u32 {
        self.bands.last().expect("bands are never empty").0
    }

    /// The bands, innermost first: `(radius in cells, fraction of the damage)`.
    ///
    /// The caller measures distance, because the metric belongs to the map's
    /// projection rather than to the definition.
    pub fn bands(&self) -> impl Iterator<Item = (u32, FixedU64)> + '_ {
        self.bands.iter().copied()
    }
}
