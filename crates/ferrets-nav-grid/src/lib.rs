//! The navigation grid: per-cell terrain, the layers over it with their
//! levels and slopes, what occupies and claims their cells, and the shapes
//! that stand on them.

pub mod cliff_crossing;
pub mod layer_id;
pub mod layer_level;
pub mod layer_mask;
pub mod mover_shape;
pub mod nav_grid;
pub mod relief;
pub mod slope;
pub mod terrain_id;
pub mod terrain_mask;
