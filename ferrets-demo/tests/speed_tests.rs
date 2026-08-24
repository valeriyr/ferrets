//! The game's speed ladder: the steps it offers, how they read, and the factors
//! it asks the engine for.

use ferrets_demo::time::{NOMINAL_TICK_HZ, SpeedStep};
use ferrets_math::FixedU64;
use ferrets_simulation::session::game_speed::GameSpeed;

#[test]
fn ladder_starts_at_normal() {
    assert_eq!(SpeedStep::default(), SpeedStep::Normal);
    assert_eq!(SpeedStep::Normal.factor(), FixedU64::ONE);
}

#[test]
fn stepping_up_and_down_saturates_at_ends() {
    assert_eq!(SpeedStep::Fastest.faster(), SpeedStep::Fastest);
    assert_eq!(SpeedStep::Slowest.slower(), SpeedStep::Slowest);
    assert_eq!(SpeedStep::Normal.faster(), SpeedStep::Fast);
    assert_eq!(SpeedStep::Normal.slower(), SpeedStep::Slow);
}

#[test]
fn stepping_up_then_down_returns_to_same_step() {
    assert_eq!(SpeedStep::Fast.faster().slower(), SpeedStep::Fast);
}

#[test]
fn every_step_lands_on_whole_cadence() {
    // Each rung against the nominal 20 Hz. Whole numbers throughout, so
    // "N ticks = M seconds" stays a clean number everywhere the game states one.
    for (step, cadence) in [
        (SpeedStep::Slowest, 5.0),
        (SpeedStep::Slow, 10.0),
        (SpeedStep::Normal, 20.0),
        (SpeedStep::Fast, 40.0),
        (SpeedStep::Faster, 80.0),
        (SpeedStep::Fastest, 160.0),
    ] {
        let factor: f64 = step.factor().to_num();
        assert_eq!(
            factor * NOMINAL_TICK_HZ,
            cadence,
            "{} ticks per second",
            step.label(),
        );
    }
}

#[test]
fn fast_forward_starts_above_double_speed() {
    // The steps a game is meant to be played at, and the ones only a watcher gets.
    assert!(!SpeedStep::Fast.fast_forward());
    assert!(SpeedStep::Faster.fast_forward());
    assert!(SpeedStep::Fastest.fast_forward());
}

#[test]
fn speed_maps_back_to_its_rung() {
    // The session is the one owner of the game's speed, so the keys and the
    // readout derive the rung from it rather than remembering one on the side.
    for step in SpeedStep::LADDER {
        assert_eq!(SpeedStep::of(step.speed()), step, "{}", step.label());
    }
    // Only the ladder can reach the session's speed — the local keys offer rungs,
    // a peer's change carries that peer's rung, and the throttle never touches it
    // (it scales the timestep) — so the lookup always finds its step. A factor
    // off the ladder is unreachable without a dishonest node, and reads as normal.
    assert_eq!(
        SpeedStep::of(GameSpeed::new(FixedU64::from_num(3))),
        SpeedStep::Normal,
        "a factor no honest node can produce falls back",
    );
}

#[test]
fn steps_ask_engine_for_their_own_factor() {
    assert_eq!(
        SpeedStep::Slowest.speed().factor(),
        FixedU64::from_num(0.25)
    );
    assert_eq!(SpeedStep::Fastest.speed().factor(), FixedU64::from_num(8));
}
