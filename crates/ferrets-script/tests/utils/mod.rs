#![allow(dead_code)]

use ferrets_content::{pool_def::PoolId, rise::RisePool};
use ferrets_math::FixedU64;

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
