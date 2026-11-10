//! Requirements as a tree: `all` and `any` nodes, and the leaves that read an entity's state.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    coverage::Coverage,
    entity_buffs::{EntityBuffDef, EntityBuffId, Lasting},
    entity_effect::EntityEffect,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    pool::Pool,
    pool_def::PoolId,
    registry::ContentRegistry,
    requirement::{Bound, Requirement, Threshold},
    skills::{Casting, EntityCastEffect, Recipient, SkillCaster, SkillDef},
    stack_rule::StackRule,
    stats::ModifierOp,
    terrains::Terrains,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_nav_grid::{layer_mask::LayerMask, terrain_mask::TerrainMask};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        last_hit,
        order_queue::OrderQueueComponent,
        pools::{self, Spending},
        stance::{Stance, StanceComponent},
    },
    entity_def,
    movement_model::MovementModel,
    order::{MoveGoal, Order},
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
        Requirement::Health(Bound::Share(Threshold::AtLeast(FixedU64::from_num(1))))
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
        Requirement::Health(Bound::Share(Threshold::AtLeast(FixedU64::from_num(1))))
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
    assert_eq!(
        pools::spend(
            app.world_mut(),
            mover,
            PoolId::ENERGY,
            FixedU64::from_num(38)
        ),
        Spending::Paid
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
    let under_twenty =
        || Requirement::Health(Bound::Amount(Threshold::Under(FixedU64::from_num(20))));

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
    utils::apply_buff(app.world_mut(), mover, doubled);
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
    utils::apply_buff(app.world_mut(), mover, slow);
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
    last_hit::record(app.world_mut(), mover, hut_id, tick);
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
                goal: MoveGoal::Position(utils::pos(25, 25)),
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
    last_hit::record(app.world_mut(), mover, SimulationId(999), hit_tick);
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
        at_sea(Coverage::Any),
    ] {
        assert!(
            !requirements::met(app.world(), 0, None, std::slice::from_ref(&leaf)),
            "{leaf:?} held with no actor"
        );
    }
}

//
// ─── Terrain under the actor ────────────────────────────────────────────────
//

#[test]
fn every_coverage_needs_whole_footprint_on_terrains() {
    let mut app = coast_app();
    let (raft, _) = utils::create_owned(&mut app, "raft", 9, 5, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::location(app.world(), raft).footprint(),
        CellRect::new(CellPos::new(9, 5), CellSize::new(2, 2)),
        "the raft lies across the coast: column 9 land, column 10 water"
    );

    assert!(!met_by(&app, raft, at_sea(Coverage::Every)));
    assert!(met_by(
        &app,
        raft,
        over(
            TerrainMask::from(utils::LAND) | utils::WATER,
            Coverage::Every
        )
    ));
}

#[test]
fn terrains_under_gather_every_occupied_cell() {
    let mut app = coast_app();
    let (raft, _) = utils::create_owned(&mut app, "raft", 9, 5, 0);
    let (ashore, _) = utils::create_owned(&mut app, "raft", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        entity_def::terrains_under(app.world(), raft),
        TerrainMask::from(utils::LAND) | utils::WATER
    );
    assert_eq!(
        entity_def::terrains_under(app.world(), ashore),
        TerrainMask::from(utils::LAND)
    );
}

#[test]
fn any_coverage_needs_one_cell_on_terrains() {
    let mut app = coast_app();
    let (raft, _) = utils::create_owned(&mut app, "raft", 9, 5, 0);
    let (ashore, _) = utils::create_owned(&mut app, "raft", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    assert!(met_by(&app, raft, at_sea(Coverage::Any)));
    assert!(!met_by(&app, ashore, at_sea(Coverage::Any)));
}

#[test]
fn over_needs_actor_on_one_of_its_layers() {
    let mut app = coast_app();
    let (swimmer, _) = utils::create_owned(&mut app, "swimmer", 12, 5, 0);
    utils::run_ticks(&mut app, 1);

    // At sea on the ground layer: met while the ground is named, not while
    // only the air is.
    let on = |layers| Requirement::Over {
        terrains: Terrains::Only(utils::WATER.into()),
        coverage: Coverage::Every,
        layers,
    };
    assert!(met_by(&app, swimmer, on(utils::GROUND.into())));
    assert!(!met_by(&app, swimmer, on(utils::AIR.into())));
    assert!(met_by(
        &app,
        swimmer,
        on(LayerMask::from(utils::GROUND) | utils::AIR)
    ));
}

#[test]
fn unless_over_holds_off_its_terrains() {
    let mut app = coast_app();
    let (swimmer, _) = utils::create_owned(&mut app, "swimmer", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    let ashore = Requirement::Unless(Box::new(at_sea(Coverage::Every)));
    assert!(met_by(&app, swimmer, ashore.clone()));
    utils::walk_onto(&mut app, swimmer, utils::pos(12, 5), 60);
    assert!(!met_by(&app, swimmer, ashore));
}

#[test]
fn actor_off_map_is_over_nothing() {
    let mut app = utils::transport_app();
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 5, 5, 0);
    let (ferry, ferry_id) = utils::create_owned(&mut app, "ferry", 6, 5, 0);
    utils::run_ticks(&mut app, 1);

    let on_map = Requirement::Over {
        terrains: Terrains::Any,
        coverage: Coverage::Every,
        layers: utils::GROUND.into(),
    };
    // Any terrain reads the same whatever the coverage.
    let touching_map = Requirement::Over {
        terrains: Terrains::Any,
        coverage: Coverage::Any,
        layers: utils::GROUND.into(),
    };
    assert!(met_by(&app, rifleman, on_map.clone()));
    assert!(met_by(&app, rifleman, touching_map.clone()));

    utils::board(&mut app, rifleman_id, ferry_id);
    utils::run_until_aboard(&mut app, ferry, 1, 40);
    assert!(!met_by(&app, rifleman, on_map.clone()));
    assert!(!met_by(&app, rifleman, touching_map));
    assert!(met_by(
        &app,
        rifleman,
        Requirement::Unless(Box::new(on_map))
    ));
}

#[test]
fn passive_over_terrain_lapses_aboard() {
    let mut app = utils::transport_app();
    let afoot = {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let afoot = registry.register_entity_buff(
            "afoot",
            EntityBuffDef {
                effects: vec![EntityEffect::Conceal],
                lasting: Lasting::While(Requirement::Over {
                    terrains: Terrains::Any,
                    coverage: Coverage::Every,
                    layers: utils::GROUND.into(),
                }),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        let mut scout = registry
            .entity("rifleman")
            .expect("the transport content declares a rifleman")
            .clone();
        scout.name = "scout".to_string();
        registry.register(scout.with_passives([afoot]));
        afoot
    };
    let (scout, scout_id) = utils::create_owned(&mut app, "scout", 5, 5, 0);
    let (ferry, ferry_id) = utils::create_owned(&mut app, "ferry", 6, 5, 0);
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), scout, afoot), "on the map");

    // Aboard it stands on nothing; the next refit drops the passive.
    utils::board(&mut app, scout_id, ferry_id);
    utils::run_until_aboard(&mut app, ferry, 1, 40);
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), scout, afoot), "aboard");
}

#[test]
fn passive_over_terrain_comes_and_goes_with_ground() {
    let mut app = coast_app();
    let (swimmer, _) = utils::create_owned(&mut app, "swimmer", 5, 5, 0);
    let wading = utils::entity_buff_id(&app, "wading");
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), swimmer, wading), "on land");

    // Out to sea and back, a tick at a time: the passive is fitted the tick
    // after the one whose move puts the swimmer's cell on the water, and
    // dropped the tick after the one that takes it off.
    utils::order_move(&mut app, swimmer, 12, 5);
    let (at_sea, fitted) = watch_crossing(&mut app, swimmer, wading, Crossing::OutToSea);
    assert_eq!(fitted, at_sea + 1);
    utils::order_move(&mut app, swimmer, 5, 5);
    let (ashore, dropped) = watch_crossing(&mut app, swimmer, wading, Crossing::Ashore);
    assert_eq!(dropped, ashore + 1);
}

#[test]
fn skill_over_terrain_is_refused_off_it() {
    let mut app = coast_app();
    let (swimmer, swimmer_id) = utils::create_owned(&mut app, "swimmer", 5, 5, 0);
    let afloat = utils::entity_buff_id(&app, "afloat");
    utils::run_ticks(&mut app, 1);

    utils::cast_self(&mut app, swimmer_id, "dive");
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(
        !entity_def::bears(app.world(), swimmer, afloat),
        "refused on land"
    );

    utils::walk_onto(&mut app, swimmer, utils::pos(12, 5), 60);
    utils::cast_self(&mut app, swimmer_id, "dive");
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(
        entity_def::bears(app.world(), swimmer, afloat),
        "cast at sea"
    );
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
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_movement(
                    utils::fixed("0.5"),
                    utils::fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(40))
                .with_pool(Pool::energy(50))
                .with_dying(2, []),
        );
        for name in ["hut", "tower"] {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(
                        utils::GROUND_DOMAIN,
                        CellSize::ONE,
                        Solidity::Solid,
                        utils::GROUND,
                    )
                    .with_pool(Pool::health(100))
                    .with_dying(2, []),
            );
        }
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    utils::start_game(&mut app);
    app
}

/// Whether player 0, acting through `actor`, meets `requirement`.
fn met_by(app: &App, actor: Entity, requirement: Requirement) -> bool {
    requirements::met(app.world(), 0, Some(actor), &[requirement])
}

/// [`utils::dry_orders_app`] with water on every cell from column 10 east: a
/// `swimmer`, the orders soldier bearing `wading` while every cell it stands
/// on is water and casting `dive` (`afloat` on itself) only there; and a
/// `raft`, a static 2×2 building with 100 health.
fn coast_app() -> App {
    let water: Vec<(u32, u32)> = (10..32)
        .flat_map(|x| (0..32).map(move |y| (x, y)))
        .collect();
    utils::dry_orders_app(MovementModel::Cell, &water, |registry, _| {
        let buff = |lasting| EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting,
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        };
        let wading =
            registry.register_entity_buff("wading", buff(Lasting::While(at_sea(Coverage::Every))));
        let afloat = registry.register_entity_buff("afloat", buff(Lasting::For(100)));
        let dive = registry.register_skill(
            "dive",
            SkillDef {
                cooldown: 1,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    casting: Casting::Instant,
                    effect: EntityCastEffect::ApplyBuff {
                        buff: afloat,
                        on: Recipient::Caster,
                    },
                },
                requires: vec![at_sea(Coverage::Every)],
            },
        );
        let mut swimmer = registry
            .entity("soldier")
            .expect("the orders content declares a soldier")
            .clone();
        swimmer.name = "swimmer".to_string();
        registry.register(swimmer.with_passives([wading]).with_skills([dive]));
        registry.register(
            EntityTypeDef::new("raft")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(2, 2),
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(100)),
        );
    })
}

/// Which way a swimmer crosses the coast.
#[derive(Clone, Copy)]
enum Crossing {
    /// From land onto the water: the buff held at sea comes.
    OutToSea,
    /// From the water onto land: the buff held at sea goes.
    Ashore,
}

/// Runs `swimmer` a tick at a time across the coast the way `crossing` says,
/// returning the tick its occupied cell first lay on the far side and the
/// tick its bearing of `buff` (held at sea) first matched that side.
fn watch_crossing(
    app: &mut App,
    swimmer: Entity,
    buff: EntityBuffId,
    crossing: Crossing,
) -> (u32, u32) {
    let at_sea_after = match crossing {
        Crossing::OutToSea => true,
        Crossing::Ashore => false,
    };
    let mut crossed = None;
    for _ in 0..80 {
        utils::run_ticks(app, 1);
        let at_sea = entity_def::location(app.world(), swimmer)
            .footprint()
            .origin
            .x
            >= 10;
        if crossed.is_none() && at_sea == at_sea_after {
            crossed = Some(utils::tick(app));
        }
        if entity_def::bears(app.world(), swimmer, buff) == at_sea_after {
            return (
                crossed.expect("the cell crosses before the buff follows"),
                utils::tick(app),
            );
        }
    }
    panic!("the swimmer crosses the coast within 80 ticks");
}

/// Standing on the ground layer over `terrains` with `coverage` of the
/// footprint.
fn over(terrains: TerrainMask, coverage: Coverage) -> Requirement {
    Requirement::Over {
        terrains: Terrains::Only(terrains),
        coverage,
        layers: utils::GROUND.into(),
    }
}

/// Standing over the coast fixture's water with `coverage` of the footprint.
fn at_sea(coverage: Coverage) -> Requirement {
    over(utils::WATER.into(), coverage)
}
