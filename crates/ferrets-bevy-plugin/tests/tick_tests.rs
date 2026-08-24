//! The tick loop's wall-clock cadence: the game states the nominal one, the
//! session's speed scales it, and the throttle lowers it to what this node and
//! its peers can actually hold.

mod utils;

use std::time::Duration;

use bevy::prelude::*;
use ferrets_bevy_plugin::{
    MIN_NOMINAL_SHARE, MIN_PEER_THROTTLE, NO_THROTTLE, NetworkActive, PauseIntent, PeerCapacities,
    SpeedIntent, TARGET_LOAD, ThrottleConfig, TickPacing,
};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    checksum,
    session::{GameSession, game_speed::GameSpeed},
};

use utils::{NOMINAL_HZ, NOMINAL_MILLIS};

/// Normal speed: the factor most of these tests measure against.
const ONE: FixedU64 = FixedU64::ONE;

//
// ─── Cadence from the chosen speed ────────────────────────────────────────────
//

#[test]
fn nominal_cadence_is_left_alone_at_normal_speed() {
    let mut app = cadence_app();

    app.update();

    assert_eq!(timestep(&app), Duration::from_millis(50));
}

#[test]
fn raising_speed_shortens_tick() {
    let mut app = cadence_app();
    set_speed(&mut app, 4);

    app.update();

    assert_eq!(timestep(&app), Duration::from_millis(50) / 4);
}

#[test]
fn lowering_speed_lengthens_tick() {
    let mut app = cadence_app();
    set_speed_factor(&mut app, fixed("0.5"));

    app.update();

    assert_eq!(timestep(&app), Duration::from_millis(100));
}

#[test]
fn returning_to_normal_restores_game_cadence() {
    // The nominal cadence is remembered, not recomputed from whatever the
    // timestep happens to be — otherwise every change would compound.
    let mut app = cadence_app();
    set_speed(&mut app, 8);
    app.update();
    set_speed(&mut app, 2);
    app.update();
    set_speed_factor(&mut app, ONE);

    app.update();

    assert_eq!(timestep(&app), Duration::from_millis(50));
}

//
// ─── Throttle ─────────────────────────────────────────────────────────────────
//

#[test]
fn tick_within_its_share_is_not_throttled() {
    // Anything up to its share of the budget — 65% of 50 ms — runs in full. The
    // boundary is stated as the share itself rather than as 32.5 ms: fixed-point
    // holds 0.65 to the last bit it has, not to the last decimal.
    assert_eq!(local(fixed("1"), ONE), NO_THROTTLE);
    assert_eq!(local(NOMINAL_MILLIS * TARGET_LOAD, ONE), NO_THROTTLE);
}

#[test]
fn tick_filling_whole_budget_is_throttled_to_target_share() {
    // Costing the entire budget leaves nothing for whatever the game draws
    // between ticks, so the cadence drops to where the cost is the share again —
    // and a tick costing four budgets drops to a quarter of that.
    assert_eq!(local(NOMINAL_MILLIS, ONE), TARGET_LOAD);
    assert_eq!(local(NOMINAL_MILLIS * 4, ONE), TARGET_LOAD / 4);
}

#[test]
fn throttling_never_speeds_game_up() {
    // The chosen speed is a ceiling: a cheap tick buys no extra cadence.
    for factor in ["0.25", "1", "8"] {
        assert_eq!(
            local(FixedU64::DELTA, fixed(factor)),
            NO_THROTTLE,
            "at {factor}x"
        );
    }
}

#[test]
fn floor_is_share_of_nominal_whatever_speed_was_chosen() {
    // The floor is a share of the cadence the game installed, divided back out by
    // the chosen speed — so however fast the game was asked to run, a hopeless
    // tick bottoms out at the same real pace. A floor expressed against the
    // chosen speed instead would shrink exactly when the budget is smallest.
    for factor in ["0.25", "1", "2", "8"] {
        let speed = fixed(factor);
        assert_eq!(
            local(fixed("10000"), speed),
            MIN_NOMINAL_SHARE / speed,
            "at {factor}x",
        );
    }
}

#[test]
fn floor_never_forces_slow_speed_faster() {
    // A cadence already below the floor is the player's choice, not a stall, so
    // the floor collapses to no throttle at all rather than speeding it up.
    let slower_than_the_floor = MIN_NOMINAL_SHARE / 2;
    assert_eq!(local(fixed("10000"), slower_than_the_floor), NO_THROTTLE);
}

//
// ─── Late frames and slow peers ───────────────────────────────────────────────
//

#[test]
fn frames_arriving_with_headroom_do_not_throttle() {
    // Any lead of a tick or more is what keeping up looks like — steady transit
    // latency lowers the lead without threatening the loop, so a frame that
    // reliably lands a full tick early constrains nothing however far it
    // travelled.
    for margin in ["1", "1.5", "2"] {
        assert_eq!(
            ferrets_bevy_plugin::throttle_for(
                fixed("1"),
                budget(ONE),
                ONE,
                Some(fixed(margin)),
                None,
            ),
            NO_THROTTLE,
            "with a lead of {margin}",
        );
    }
}

#[test]
fn margin_under_headroom_eases_cadence_off() {
    // Half a tick of headroom halves the cadence, handing the late peer twice
    // the wall time per tick.
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(fixed("1"), budget(ONE), ONE, Some(fixed("0.5")), None),
        fixed("0.5"),
    );
    // No lead at all stops there rather than dropping to the floor: frames that
    // simply are not arriving are what blocking and the drop rule are for.
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(fixed("1"), budget(ONE), ONE, Some(FixedU64::ZERO), None),
        MIN_PEER_THROTTLE,
    );
}

#[test]
fn peer_that_cannot_hold_cadence_caps_it() {
    // At 1x a peer good for only 0.8x pulls everyone to 0.8x, while one good for
    // more than was chosen constrains nothing.
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(fixed("1"), budget(ONE), ONE, None, Some(fixed("0.8"))),
        fixed("0.8"),
    );
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(fixed("1"), budget(ONE), ONE, None, Some(fixed("4"))),
        NO_THROTTLE,
    );
}

#[test]
fn peer_beyond_accommodating_is_left_to_drop_rule() {
    // A peer claiming a tenth of the cadence does not get to hold the game there.
    // Accommodation stops at half, after which the peer falls behind, blocks the
    // tick, and the drop rule decides — it has the grace window, the authority and
    // the game's own say, none of which a throttle has any business guessing at.
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(fixed("1"), budget(ONE), ONE, None, Some(fixed("0.1"))),
        MIN_PEER_THROTTLE,
    );
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(
            fixed("1"),
            budget(ONE),
            ONE,
            Some(FixedU64::ZERO),
            Some(fixed("0.1")),
        ),
        MIN_PEER_THROTTLE,
        "two peer troubles together are no worse than one",
    );
}

#[test]
fn own_cost_floor_is_deeper_than_peer_floor() {
    // Nobody can be dropped to fix this node's own tick cost, so slowing down is
    // the only recourse there is and the floor goes deeper.
    let hopeless = local(fixed("10000"), ONE);
    assert_eq!(hopeless, MIN_NOMINAL_SHARE);
    assert!(
        hopeless < MIN_PEER_THROTTLE,
        "own trouble may cost more than someone else's",
    );
}

#[test]
fn peer_cap_is_read_against_chosen_speed() {
    // A peer good for 2x is no constraint at 1x, but at 4x it is half of what was
    // asked for.
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(
            FixedU64::DELTA,
            budget(ONE),
            ONE,
            None,
            Some(fixed("2"))
        ),
        NO_THROTTLE,
    );
    let quick = fixed("4");
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(
            FixedU64::DELTA,
            budget(quick),
            quick,
            None,
            Some(fixed("2"))
        ),
        fixed("0.5"),
    );
}

#[test]
fn tightest_of_three_wins() {
    // Local cost, delivery and the slowest peer are three ways to be short of
    // budget; the throttle answers to whichever is tightest.
    let full = budget(ONE);
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(full, full, ONE, Some(fixed("2")), Some(fixed("4"))),
        fixed("0.65"),
        "this node's own cost",
    );
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(
            FixedU64::DELTA,
            full,
            ONE,
            Some(fixed("0.6")),
            Some(fixed("4"))
        ),
        fixed("0.6"),
        "delivery",
    );
    assert_eq!(
        ferrets_bevy_plugin::throttle_for(
            FixedU64::DELTA,
            full,
            ONE,
            Some(fixed("2")),
            Some(fixed("0.2"))
        ),
        MIN_PEER_THROTTLE,
        "the slowest peer",
    );
}

//
// ─── Sustainable factor ───────────────────────────────────────────────────────
//

#[test]
fn sustainable_factor_reads_tick_cost_against_nominal_budget() {
    // A tick costing a whole nominal tick can be held at the target share of
    // normal speed; twice that at half of it, a quarter of it at four times.
    assert_eq!(
        ferrets_bevy_plugin::sustainable_factor(NOMINAL_MILLIS, NOMINAL_MILLIS),
        TARGET_LOAD
    );
    assert_eq!(
        ferrets_bevy_plugin::sustainable_factor(NOMINAL_MILLIS * 2, NOMINAL_MILLIS),
        TARGET_LOAD / 2,
    );
    assert_eq!(
        ferrets_bevy_plugin::sustainable_factor(NOMINAL_MILLIS / 4, NOMINAL_MILLIS),
        TARGET_LOAD * 4,
    );
}

#[test]
fn sustainable_factor_halves_when_tick_cost_doubles() {
    // What this node can hold is a statement about the tick's cost alone — never
    // about the cadence in force, which a throttle would already have lowered.
    let cost = fixed("20");
    assert_eq!(
        ferrets_bevy_plugin::sustainable_factor(cost * 2, NOMINAL_MILLIS),
        ferrets_bevy_plugin::sustainable_factor(cost, NOMINAL_MILLIS) / 2,
    );
}

//
// ─── Opting out ───────────────────────────────────────────────────────────────
//

#[test]
fn game_that_turns_throttling_off_keeps_its_chosen_cadence() {
    // A game may prefer to fall behind in hitches rather than change pace.
    let mut app = cadence_app();
    app.world_mut().resource_mut::<ThrottleConfig>().enabled = false;
    utils::set_tick_cost(&mut app, fixed("10000"));

    app.update();

    assert_eq!(timestep(&app), Duration::from_millis(50));
    assert_eq!(throttle(&app), NO_THROTTLE);
}

#[test]
fn throttling_on_reacts_to_same_tick_cost() {
    // The same app with the default config: the hopeless tick bottoms the cadence
    // out at the floor, so the assertion above is about the switch and not about
    // the cost being ignored anyway.
    let mut app = cadence_app();
    utils::set_tick_cost(&mut app, fixed("10000"));

    app.update();

    assert_eq!(throttle(&app), MIN_NOMINAL_SHARE);
}

#[test]
fn game_that_turns_sharing_off_ignores_what_peers_report() {
    // Opting out of sharing means neither publishing nor being constrained; a
    // report that arrived anyway leaves the cadence alone.
    let mut app = cadence_app();
    app.world_mut()
        .resource_mut::<ThrottleConfig>()
        .share_capacity = false;
    report_capacity(&mut app, fixed("0.75"));

    app.update();

    assert_eq!(throttle(&app), NO_THROTTLE);
}

#[test]
fn sharing_on_takes_same_report_as_cap() {
    let mut app = cadence_app();
    report_capacity(&mut app, fixed("0.75"));

    app.update();

    assert_eq!(throttle(&app), fixed("0.75"));
}

//
// ─── Pause and speed intents ──────────────────────────────────────────────────
//

#[test]
fn local_speed_intent_sets_session_speed() {
    // Off the network the intents are the game's only way to steer the session;
    // the engine applies them ahead of the frame's fixed loop.
    let mut app = cadence_app();
    app.world_mut().resource_mut::<SpeedIntent>().0 = Some(GameSpeed::new(fixed("2")));

    app.update();
    assert_eq!(
        app.world().resource::<GameSession>().speed(),
        GameSpeed::new(fixed("2")),
    );

    // The cadence follows on the next frame: intents apply in `PreUpdate`, after
    // the frame's own `First` has already derived the timestep.
    app.update();
    assert_eq!(timestep(&app), Duration::from_millis(25));
}

#[test]
fn local_pause_intent_pauses_session() {
    let mut app = cadence_app();
    app.world_mut().resource_mut::<PauseIntent>().0 = Some(true);

    app.update();

    assert!(app.world().resource::<GameSession>().is_paused());
}

#[test]
fn networked_game_leaves_intents_to_control_plane() {
    // With a network session installed the same intents become tick-aligned
    // changes through the session's authority (`net_control`); applying them
    // here would put this node on a different cadence than its peers.
    let mut app = cadence_app();
    app.world_mut().insert_resource(NetworkActive);
    app.world_mut().resource_mut::<SpeedIntent>().0 = Some(GameSpeed::new(fixed("2")));

    app.update();

    assert_eq!(
        app.world().resource::<GameSession>().speed(),
        GameSpeed::NORMAL,
        "the local applier stood down",
    );
    assert_eq!(
        app.world().resource::<SpeedIntent>().0,
        Some(GameSpeed::new(fixed("2"))),
        "the intent is left for the control plane",
    );
}

//
// ─── Only a running game is paced ─────────────────────────────────────────────
//

#[test]
fn paused_game_is_not_throttled() {
    // Nothing is advancing, so there is no cadence to spread out — and a
    // stretched step would make the drop rule's grace window, counted in blocked
    // steps, take that many times longer in wall time to expire.
    let mut app = cadence_app();
    utils::set_tick_cost(&mut app, fixed("10000"));
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_paused(true);

    app.update();

    assert_eq!(throttle(&app), NO_THROTTLE);
    assert_eq!(timestep(&app), Duration::from_millis(50));
}

#[test]
fn manually_driven_tick_is_not_measured() {
    // A seek, a single step and a headless run all drive ticks back to back with
    // nothing drawn between them, so what they cost says nothing about holding a
    // cadence.
    let mut app = cadence_app();
    let before = app.world().resource::<TickPacing>().exec_millis;
    let tick = utils::tick(&app);

    ferrets_bevy_plugin::run_tick(app.world_mut());

    assert!(
        utils::tick(&app) > tick,
        "the tick did advance, so only the manual rule can be what skipped it",
    );
    assert_eq!(app.world().resource::<TickPacing>().exec_millis, before);
}

//
// ─── Cadence cannot reach the simulation ──────────────────────────────────────
//

#[test]
fn nodes_at_different_cadences_compute_identical_state() {
    // The cadence is measured from a wall clock, which is exactly what a lockstep
    // simulation may not depend on. It is safe only because none of it reaches the
    // tick: two nodes running the same input at wildly different speeds, one of
    // them throttled to its floor, must agree bit for bit.
    let mut quick = cadence_app();
    let mut slow = cadence_app();
    set_speed(&mut quick, 8);
    set_speed_factor(&mut slow, fixed("0.25"));
    // Set the pacing rather than provoking it: the throttle is computed in
    // `First`, which driving ticks by hand does not run, and how it is *derived*
    // is what the tests above are for.
    slow.world_mut().resource_mut::<TickPacing>().throttle = MIN_NOMINAL_SHARE;

    for _ in 0..30 {
        ferrets_bevy_plugin::run_tick(quick.world_mut());
        ferrets_bevy_plugin::run_tick(slow.world_mut());
    }

    assert_ne!(
        throttle(&quick),
        throttle(&slow),
        "the two really are pacing differently",
    );
    assert_eq!(utils::tick(&quick), utils::tick(&slow));
    assert_eq!(
        checksum::state_checksum(quick.world()),
        checksum::state_checksum(slow.world()),
        "cadence is not simulation state",
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A factor or throttle, spelled the way a test wants to read it.
fn fixed(value: &str) -> FixedU64 {
    value.parse().expect("a fixed-point value")
}

/// The budget one tick has at `speed` times the nominal cadence, in milliseconds.
fn budget(speed: FixedU64) -> FixedU64 {
    NOMINAL_MILLIS / speed
}

/// The throttle from this node's own tick cost, with no peers in the picture.
fn local(exec_millis: FixedU64, speed: FixedU64) -> FixedU64 {
    ferrets_bevy_plugin::throttle_for(exec_millis, budget(speed), speed, None, None)
}

fn throttle(app: &App) -> FixedU64 {
    app.world().resource::<TickPacing>().throttle
}

fn timestep(app: &App) -> Duration {
    app.world().resource::<Time<Fixed>>().timestep()
}

fn set_speed(app: &mut App, factor: u32) {
    set_speed_factor(app, FixedU64::from_num(factor));
}

fn set_speed_factor(app: &mut App, factor: FixedU64) {
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_speed(GameSpeed::new(factor));
}

/// Records a peer as having reported that it can sustain `capacity`.
fn report_capacity(app: &mut App, capacity: FixedU64) {
    app.world_mut()
        .resource_mut::<PeerCapacities>()
        .record(1, 0, GameSpeed::new(capacity));
}

/// An app whose frames actually run, with the game's own cadence installed.
fn cadence_app() -> App {
    let mut app = utils::make_app(utils::human_slots(1));
    app.add_plugins(MinimalPlugins);
    app.insert_resource(Time::<Fixed>::from_hz(NOMINAL_HZ));
    // Normally the network plugin's; the cadence reads it when peers have
    // reported, and a game without networking simply never fills it.
    app.init_resource::<PeerCapacities>();
    // Only a running game is paced, so a pending one would report no throttle
    // whatever it was measuring.
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
