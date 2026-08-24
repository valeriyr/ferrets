//! The game speed factor: what a session's cadence may be scaled by.

use ferrets_math::FixedU64;
use ferrets_simulation::session::game_speed::{GameSpeed, error::GameSpeedError};

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_carries_factor() {
    let quarter = FixedU64::from_num(0.25);
    assert_eq!(GameSpeed::new(quarter).factor(), quarter);
}

#[test]
fn default_speed_is_normal() {
    assert_eq!(GameSpeed::default(), GameSpeed::NORMAL);
    assert_eq!(GameSpeed::NORMAL.factor(), FixedU64::ONE);
}

#[test]
#[should_panic(expected = "a game speed factor cannot be zero")]
fn zero_factor_is_refused() {
    GameSpeed::new(FixedU64::ZERO);
}

//
// ─── Fallible construction ────────────────────────────────────────────────────
//

#[test]
fn try_from_accepts_positive_factor() {
    let speed = GameSpeed::try_from(FixedU64::from_num(2)).expect("nonzero factor is valid");
    assert_eq!(speed.factor(), FixedU64::from_num(2));
}

#[test]
fn try_from_refuses_zero_factor() {
    // The fallible path exists for values arriving from outside the program,
    // so the refusal is an error naming the violation, never a panic.
    let error = GameSpeed::try_from(FixedU64::ZERO).expect_err("zero factor is refused");
    assert!(matches!(error, GameSpeedError::ZeroSpeedFactor));
}
