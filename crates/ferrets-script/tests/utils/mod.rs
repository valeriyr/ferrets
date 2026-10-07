#![allow(dead_code)]

use ferrets_content::{build::SitePool, pool_def::PoolId};
use ferrets_math::FixedU64;

/// A fixed-point value parsed from decimal digits.
pub fn fixed(text: &str) -> FixedU64 {
    text.parse()
        .unwrap_or_else(|_| panic!("'{text}' is a value"))
}

/// Each of `pools` held on a site from its own initial.
pub fn site_initial(pools: &[PoolId]) -> Vec<(PoolId, SitePool)> {
    pools
        .iter()
        .map(|pool| (*pool, SitePool::Initial))
        .collect()
}
