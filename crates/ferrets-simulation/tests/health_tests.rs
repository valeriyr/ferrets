//! Tests for the health pool's integer projection at the life/death boundary.

use ferrets_math::FixedU64;
use ferrets_simulation::components::health::HealthComponent;

//
// ─── Displayed points ─────────────────────────────────────────────────────────
//

#[test]
fn full_pool_displays_maximum() {
    let health = HealthComponent::full(FixedU64::from_num(50));
    assert_eq!(health.displayed(), 50);
}

#[test]
fn sub_point_damage_stays_visible() {
    // 50 - 0.5 = 49.5: a fraction of a point lost still reads as lost (floor),
    // never rounded back up to full.
    let mut health = HealthComponent::full(FixedU64::from_num(50));
    health.drain(FixedU64::from_num(0.5));
    assert_eq!(health.displayed(), 49);
}

#[test]
fn barely_alive_pool_never_displays_zero() {
    // 1 - 0.5 = 0.5: below a whole point but still alive, so it reads as 1, not 0.
    let mut health = HealthComponent::full(FixedU64::from_num(1));
    health.drain(FixedU64::from_num(0.5));
    assert!(!health.is_dead());
    assert_eq!(health.displayed(), 1);
}

#[test]
fn dead_pool_displays_zero() {
    let mut health = HealthComponent::full(FixedU64::from_num(1));
    health.drain(FixedU64::from_num(1));
    assert!(health.is_dead());
    assert_eq!(health.displayed(), 0);
}
