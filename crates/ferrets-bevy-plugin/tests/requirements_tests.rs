//! Requirements as a tree: `all` and `any` nodes, and the leaves that read an entity's state.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    registry::ContentRegistry,
    requirement::{Bound, Requirement, Threshold},
    stats::ModifierOp,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        energy::EnergyComponent,
        health::HealthComponent,
        order_queue::OrderQueueComponent,
        stance::{Stance, StanceComponent},
    },
    game_loop,
    order::Order,
    requirements,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
};

//
// ─── Nodes ──────────────────────────────────────────────────────────────────
//

#[test]
fn any_node_is_met_by_either_branch() {
    let mut app = app();
    utils::create_owned(&mut app, "hut", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    let either = Requirement::Any(vec![
        Requirement::EntityType("hut".to_string()),
        Requirement::EntityType("tower".to_string()),
    ]);
    assert!(requirements::met(app.world(), 0, None, &[either]));
}

#[test]
fn any_node_is_unmet_with_no_branch() {
    let mut app = app();
    utils::run_ticks(&mut app, 1);

    let either = Requirement::Any(vec![
        Requirement::EntityType("hut".to_string()),
        Requirement::EntityType("tower".to_string()),
    ]);
    assert!(!requirements::met(app.world(), 0, None, &[either]));
}

#[test]
fn all_node_needs_every_branch() {
    let mut app = app();
    utils::create_owned(&mut app, "hut", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    let both = Requirement::All(vec![
        Requirement::EntityType("hut".to_string()),
        Requirement::EntityType("tower".to_string()),
    ]);
    assert!(!requirements::met(
        app.world(),
        0,
        None,
        std::slice::from_ref(&both)
    ));

    utils::create_owned(&mut app, "tower", 8, 8, 0);
    utils::run_ticks(&mut app, 1);
    assert!(requirements::met(app.world(), 0, None, &[both]));
}

#[test]
fn nodes_nest() {
    let mut app = app();
    utils::create_owned(&mut app, "tower", 8, 8, 0);
    utils::run_ticks(&mut app, 1);

    // all { any { hut, tower }, tower }: the inner any is met by the tower,
    // and so is the outer all's second branch.
    let nested = Requirement::All(vec![
        Requirement::Any(vec![
            Requirement::EntityType("hut".to_string()),
            Requirement::EntityType("tower".to_string()),
        ]),
        Requirement::EntityType("tower".to_string()),
    ]);
    assert!(requirements::met(app.world(), 0, None, &[nested]));
}

//
// ─── Pools ──────────────────────────────────────────────────────────────────
//

#[test]
fn health_line_is_strict_under_and_inclusive_at_least() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // 40 of 40: at the whole, so at_least 1 holds and under 0.5 does not.
    assert!(met_by(
        &app,
        mover,
        Requirement::Health(Bound::Share(Threshold::AtLeast(utils::fixed("1"))))
    ));
    assert!(!met_by(
        &app,
        mover,
        Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.5"))))
    ));

    // 20 of 40: exactly on the half line — not under it, at least it.
    utils::wound(&mut app, mover, "20");
    assert!(!met_by(
        &app,
        mover,
        Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.5"))))
    ));
    assert!(met_by(
        &app,
        mover,
        Requirement::Health(Bound::Share(Threshold::AtLeast(utils::fixed("0.5"))))
    ));
    assert!(!met_by(
        &app,
        mover,
        Requirement::Health(Bound::Share(Threshold::AtLeast(utils::fixed("1"))))
    ));

    // 19 of 40: under the half line.
    utils::wound(&mut app, mover, "1");
    assert!(met_by(
        &app,
        mover,
        Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.5"))))
    ));
}

#[test]
fn energy_line_reads_energy_pool() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // 50 of 50 energy: not under a quarter.
    assert!(!met_by(
        &app,
        mover,
        Requirement::Energy(Bound::Share(Threshold::Under(utils::fixed("0.25"))))
    ));

    // Spent down to 12 of 50: 12 < 12.5, under a quarter.
    assert!(
        app.world_mut()
            .get_mut::<EnergyComponent>(mover)
            .unwrap()
            .spend(utils::fixed("38"))
    );
    assert!(met_by(
        &app,
        mover,
        Requirement::Energy(Bound::Share(Threshold::Under(utils::fixed("0.25"))))
    ));
}

#[test]
fn health_amount_reads_points() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let under_twenty = || Requirement::Health(Bound::Amount(Threshold::Under(utils::fixed("20"))));

    // 20 of 40 is on the line, not under it; 19 is under.
    utils::wound(&mut app, mover, "20");
    assert!(!met_by(&app, mover, under_twenty()));
    utils::wound(&mut app, mover, "1");
    assert!(met_by(&app, mover, under_twenty()));
}

#[test]
fn pool_share_reads_effective_maximum() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    let doubled = utils::register_entity_buff(
        &mut app,
        "doubled",
        EntityStatId::MAX_HEALTH,
        ModifierOp::PercentAdd,
        "1",
        None,
    );
    game_loop::stats::apply_entity_buff(app.world_mut(), mover, doubled);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, mover, "10");

    // 30 of an effective 80: under half of it (40). Against the base 40 the
    // line would be 20, and 30 is not under that.
    assert!(met_by(
        &app,
        mover,
        Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.5"))))
    ));
}

#[test]
fn pool_line_on_entity_without_that_pool_is_unmet() {
    let mut app = app();
    let (hut, _) = utils::create_owned(&mut app, "hut", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The hut has health but no energy: neither side of an energy line holds.
    assert!(!met_by(
        &app,
        hut,
        Requirement::Energy(Bound::Share(Threshold::Under(utils::fixed("0.5"))))
    ));
    assert!(!met_by(
        &app,
        hut,
        Requirement::Energy(Bound::Share(Threshold::AtLeast(utils::fixed("0.5"))))
    ));
}

//
// ─── Stats, idleness and hits ───────────────────────────────────────────────
//

#[test]
fn stat_line_reads_effective_stat() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Speed is 0.5: under 0.6, at least 0.5, not under 0.5.
    let speed = |bound| Requirement::Stat {
        stat: EntityStatId::SPEED,
        bound,
    };
    assert!(met_by(
        &app,
        mover,
        speed(Bound::Amount(Threshold::Under(utils::fixed("0.6"))))
    ));
    assert!(met_by(
        &app,
        mover,
        speed(Bound::Amount(Threshold::AtLeast(utils::fixed("0.5"))))
    ));
    assert!(!met_by(
        &app,
        mover,
        speed(Bound::Amount(Threshold::Under(utils::fixed("0.5"))))
    ));
}

#[test]
fn stat_share_reads_effective_stat_against_base() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let slowed = || Requirement::Stat {
        stat: EntityStatId::SPEED,
        bound: Bound::Share(Threshold::Under(utils::fixed("0.7"))),
    };

    // At its base 0.5 the line is 70% of 0.5 = 0.35: not under it.
    assert!(!met_by(&app, mover, slowed()));

    // A 40% slow takes it to 0.3, under 0.35; the base, and so the line, stays.
    let slow = utils::register_entity_buff(
        &mut app,
        "slow",
        EntityStatId::SPEED,
        ModifierOp::PercentAdd,
        "-0.4",
        None,
    );
    game_loop::stats::apply_entity_buff(app.world_mut(), mover, slow);
    utils::run_ticks(&mut app, 1);
    assert!(met_by(&app, mover, slowed()));
}

#[test]
fn idle_follows_order_queue() {
    let mut app = app();
    let (mover, mover_id) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    assert!(met_by(&app, mover, Requirement::Idle));

    utils::select(&mut app, mover_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(25, 25),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!met_by(&app, mover, Requirement::Idle));

    utils::force_cancel_orders(app.world_mut(), mover);
    utils::run_ticks(&mut app, 1);
    assert!(met_by(&app, mover, Requirement::Idle));
}

#[test]
fn idle_for_counts_ticks_since_queue_emptied() {
    let mut app = app();
    let (mover, mover_id) = utils::create_owned(&mut app, "mover", 5, 5, 0);

    // Its queue is found empty at the end of the first tick's orders: one
    // tick idle once that tick is over, five after four more.
    utils::run_ticks(&mut app, 1);
    assert!(met_by(&app, mover, Requirement::IdleFor(1)));
    assert!(!met_by(&app, mover, Requirement::IdleFor(5)));
    utils::run_ticks(&mut app, 3);
    assert!(!met_by(&app, mover, Requirement::IdleFor(5)));
    utils::run_ticks(&mut app, 1);
    assert!(met_by(&app, mover, Requirement::IdleFor(5)));

    // An order makes it busy: not idle for even one tick.
    utils::select(&mut app, mover_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(25, 25),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!met_by(&app, mover, Requirement::IdleFor(1)));

    // Stopped, the count starts over from the tick it stood empty again.
    utils::force_cancel_orders(app.world_mut(), mover);
    utils::run_ticks(&mut app, 1);
    assert!(met_by(&app, mover, Requirement::IdleFor(1)));
    assert!(!met_by(&app, mover, Requirement::IdleFor(2)));
}

#[test]
fn order_ending_in_its_own_tick_resets_idle_for() {
    let mut app = app();
    let (mover, mover_id) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 6);
    assert!(met_by(&app, mover, Requirement::IdleFor(5)));

    // A move to where it stands starts and ends in one tick; it still counts
    // as an order, so the count starts over.
    let here = utils::position_of(app.world(), mover);
    utils::select(&mut app, mover_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: here,
            flush: true,
        },
    );
    // The count restarted: two ticks idle by the end of the run, where an
    // unbroken count would stand at 6 + 4 = 10.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(met_by(&app, mover, Requirement::IdleFor(2)));
    assert!(!met_by(&app, mover, Requirement::IdleFor(3)));
    utils::run_ticks(&mut app, 3);
    assert!(met_by(&app, mover, Requirement::IdleFor(5)));
    assert!(!met_by(&app, mover, Requirement::IdleFor(6)));
}

#[test]
fn flight_ending_in_its_own_tick_resets_idle_for() {
    let mut app = app();
    // In the map's far corner, a hit from nearer the origin sends it fleeing
    // toward a cell clamped to its own, so the flight ends the tick it is
    // given.
    let (mover, _) = utils::create_owned(&mut app, "mover", 31, 31, 0);
    let (_, hut_id) = utils::create_owned(&mut app, "hut", 20, 20, 0);
    app.world_mut().get_mut::<StanceComponent>(mover).unwrap().0 = Stance::Flee;
    utils::run_ticks(&mut app, 6);
    assert!(met_by(&app, mover, Requirement::IdleFor(5)));

    let tick = app.world().resource::<GameSession>().tick();
    app.world_mut()
        .get_mut::<HealthComponent>(mover)
        .unwrap()
        .record_hit(hut_id, tick);
    // The count restarted: two ticks idle by the end of the run, where an
    // unbroken count would stand at 6 + 3 = 9.
    utils::run_ticks(&mut app, 3);
    assert!(met_by(&app, mover, Requirement::IdleFor(2)));
    assert!(!met_by(&app, mover, Requirement::IdleFor(3)));
}

#[test]
fn idle_for_needs_empty_queue_now() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 5);
    assert!(met_by(&app, mover, Requirement::IdleFor(5)));

    // An order in the queue ends it at once, before the tick that notes it.
    app.world_mut()
        .get_mut::<OrderQueueComponent>(mover)
        .unwrap()
        .push(
            Order::Move {
                target: utils::pos(25, 25),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );
    assert!(!met_by(&app, mover, Requirement::IdleFor(1)));
    assert!(!met_by(&app, mover, Requirement::Idle));
}

#[test]
fn entity_without_order_queue_is_never_idle() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 3);
    app.world_mut()
        .entity_mut(mover)
        .remove::<OrderQueueComponent>();

    assert!(!met_by(&app, mover, Requirement::Idle));
    assert!(!met_by(&app, mover, Requirement::IdleFor(1)));
}

#[test]
fn unhurt_for_counts_ticks_since_last_hit() {
    let mut app = app();
    let (mover, _) = utils::create_owned(&mut app, "mover", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Never hit: unhurt for any stretch.
    assert!(met_by(&app, mover, Requirement::UnhurtFor(5)));

    let hit_tick = utils::tick(&app);
    app.world_mut()
        .get_mut::<HealthComponent>(mover)
        .unwrap()
        .record_hit(SimulationId(999), hit_tick);
    assert!(!met_by(&app, mover, Requirement::UnhurtFor(5)));

    // Four ticks on: 4 < 5, still hurt. Five on: unhurt for five.
    utils::run_ticks(&mut app, 4);
    assert!(!met_by(&app, mover, Requirement::UnhurtFor(5)));
    utils::run_ticks(&mut app, 1);
    assert!(met_by(&app, mover, Requirement::UnhurtFor(5)));
}

#[test]
fn actor_leaves_are_unmet_without_actor() {
    let mut app = app();
    utils::run_ticks(&mut app, 1);

    for leaf in [
        Requirement::Health(Bound::Share(Threshold::AtLeast(utils::fixed("0.1")))),
        Requirement::Energy(Bound::Share(Threshold::AtLeast(utils::fixed("0.1")))),
        Requirement::Stat {
            stat: EntityStatId::SPEED,
            bound: Bound::Amount(Threshold::AtLeast(utils::fixed("0.1"))),
        },
        Requirement::Idle,
        Requirement::IdleFor(1),
        Requirement::UnhurtFor(1),
    ] {
        assert!(
            !requirements::met(app.world(), 0, None, std::slice::from_ref(&leaf)),
            "{leaf:?} held with no actor"
        );
    }
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// One human player; a `mover` with 40 health, 50 energy and speed 0.5, and
/// two buildings, `hut` and `tower`, with health alone.
fn app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("mover")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    utils::fixed("0.5"),
                    utils::fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(40)
                .with_energy(50, FixedU64::ZERO)
                .with_dying(2, []),
        );
        for name in ["hut", "tower"] {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                    .with_health(100)
                    .with_dying(2, []),
            );
        }
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Whether player 0, acting through `actor`, meets `requirement`.
fn met_by(app: &App, actor: Entity, requirement: Requirement) -> bool {
    requirements::met(app.world(), 0, Some(actor), &[requirement])
}
