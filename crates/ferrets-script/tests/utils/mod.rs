#![allow(dead_code)]

use ferrets_content::{domain::DomainId, pool_def::PoolId, rise::RisePool};
use ferrets_math::FixedU64;
use ferrets_nav_grid::{layer_id::LayerId, terrain_id::TerrainId};

/// The layers the test content declares, in the order a registry mints them:
/// the first layer declared is `1`, and each next one takes the next bit.
pub const GROUND: LayerId = LayerId::new(1);
pub const AIR: LayerId = LayerId::new(2);

/// The terrains the test content declares, minted the same way.
pub const LAND: TerrainId = TerrainId::new(1);
pub const SHALLOW_WATER: TerrainId = TerrainId::new(2);

/// The domain the test content declares first.
pub const GROUND_DOMAIN: DomainId = DomainId::new(0);

/// A fixed-point value parsed from decimal digits.
pub fn fixed(text: &str) -> FixedU64 {
    text.parse()
        .unwrap_or_else(|_| panic!("'{text}' is a value"))
}

/// Each of `pools` held at its own initial while rising.
pub fn rise_initial_pools(pools: &[PoolId]) -> Vec<(PoolId, RisePool)> {
    pools
        .iter()
        .map(|pool| (*pool, RisePool::Initial))
        .collect()
}
