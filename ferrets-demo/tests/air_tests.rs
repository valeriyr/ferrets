//! The air layer in play, on the demo skirmish map and content: what a flier
//! may cross, where it may come to rest, and who it shares a cell with.
//!
//! These run on the 96×96 demo map rather than the built-in mission, because
//! the mission's map is 32×32 and all grass — it has no water to fly over.

mod utils;

use bevy::prelude::{App, Entity, World};
use ferrets_content::{entity_stats::EntityStatId, registry::ContentRegistry};
use ferrets_demo::map;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    components::{
        health::HealthComponent, hidden::HiddenComponent, order_queue::OrderQueueComponent,
        transport::TransporterComponent,
    },
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
    order::Order,
    simulation_id::SimulationId,
    spawn,
};

//
// ─── Crossing and resting ──────────────────────────────────────────────────────
//

#[test]
fn flier_crosses_river_and_rests_on_water() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    let (flier, _) = utils::create_entity(
        app.world_mut(),
        "gryphon_aloft",
        utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1),
        Some(0),
    )
    .expect("the flier spawns over clear ground");

    // The river splits the map for anything on foot, and this stretch has no
    // ford: a walker would have to detour to y 18..20. The flier goes straight
    // over and stops on the water.
    app.world_mut()
        .entity_mut(flier)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: utils::at_cell(RIVER.0, RIVER.1),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 300);

    let world = app.world_mut();
    assert!(
        world
            .entity(flier)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the flight never finished, so the air layer did not carry it over the river"
    );
    let landed = utils::cell_of(utils::position_of(world, flier));
    assert_eq!(
        landed, RIVER,
        "the flier rested at {landed:?} instead of on the ordered water cell"
    );
}

#[test]
fn walker_cannot_stand_where_flier_rests() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // The same cell the flier settles on above: proof that the destination is
    // genuinely closed to the ground layer, so the flight test is not just
    // walking across open grass.
    assert!(
        utils::create_entity(
            app.world_mut(),
            "peasant",
            utils::at_cell(RIVER.0, RIVER.1),
            Some(0),
        )
        .is_none(),
        "a walker was placed on the river, so it is not blocked ground after all"
    );
}

//
// ─── Multi-layer occupation: what is tall enough to be in the way ──────────────
//

#[test]
fn ground_building_does_not_block_flight() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let cell = utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1);

    // A 2x2 farm holds the ground layer only, so the air over it stays open.
    utils::create_entity(app.world_mut(), "farm", cell, Some(0)).expect("the farm is raised");

    let world = app.world();
    let registry = world.resource::<ContentRegistry>();
    let ground = registry.layer(map::GROUND).unwrap();
    let air = registry.layer(map::AIR).unwrap();
    let live = world.resource::<Map>();

    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let footprint = CellPos::new(CLEAR_GROUND.0 + dx, CLEAR_GROUND.1 + dy);
        assert!(
            !live.nav_grid().is_passable(ground, footprint),
            "the farm does not hold its own ground at {footprint:?}"
        );
        assert!(
            live.nav_grid().is_passable(air, footprint),
            "the farm closed the air at {footprint:?}, but it occupies the ground alone"
        );
    }
}

#[test]
fn tall_building_blocks_flight() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // The fortress holds the water beneath it and the air above it at once, so
    // its footprint is closed to ships and fliers alike.
    let origin = (46, 46);
    utils::create_entity(
        app.world_mut(),
        "sea_fortress",
        utils::at_cell(origin.0, origin.1),
        None,
    )
    .expect("the fortress stands on open water");

    let world = app.world();
    let registry = world.resource::<ContentRegistry>();
    let water = registry.layer(map::WATER).unwrap();
    let air = registry.layer(map::AIR).unwrap();
    let live = world.resource::<Map>();

    for dy in 0..3 {
        for dx in 0..3 {
            let footprint = CellPos::new(origin.0 + dx, origin.1 + dy);
            assert!(
                !live.nav_grid().is_passable(water, footprint),
                "the fortress does not hold the water at {footprint:?}"
            );
            assert!(
                !live.nav_grid().is_passable(air, footprint),
                "the fortress left the air open at {footprint:?}, so nothing on the map \
                 obstructs flight"
            );
        }
    }
}

#[test]
fn flier_routes_around_tall_building() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // Standing on open water beside the fortress, ordered to the water on its
    // far side: the only way through is around, and the flight must still finish.
    let fortress = (46, 46);
    utils::create_entity(
        app.world_mut(),
        "sea_fortress",
        utils::at_cell(fortress.0, fortress.1),
        None,
    )
    .expect("the fortress stands");

    let start = (44, 47);
    // Clear of the keep's own five cells, which start at its corner.
    let goal = (53, 47);
    let (flier, _) = utils::create_entity(
        app.world_mut(),
        "gryphon_aloft",
        utils::at_cell(start.0, start.1),
        Some(0),
    )
    .expect("the flier spawns beside the fortress");
    app.world_mut()
        .entity_mut(flier)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: utils::at_cell(goal.0, goal.1),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    // Sample every tick: a straight line from start to goal runs through the
    // fortress, so passing over it would be indistinguishable from a detour if
    // only the arrival were checked. The flier is 2x2, so the whole anchored
    // footprint must stay off the fortress rect, not the anchor alone.
    for _ in 0..300 {
        utils::run_ticks(&mut app, 1);
        let (x, y) = utils::cell_of(utils::position_of(app.world_mut(), flier));
        let overlaps = x <= fortress.0 + 2
            && x + 1 >= fortress.0
            && y <= fortress.1 + 2
            && y + 1 >= fortress.1;
        assert!(
            !overlaps,
            "the flier passed over the fortress at ({x}, {y}) instead of rounding it"
        );
    }

    let world = app.world_mut();
    assert!(
        world
            .entity(flier)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the flight around the fortress never finished"
    );
    let landed = utils::cell_of(utils::position_of(world, flier));
    assert_eq!(
        landed, goal,
        "the flier rested at {landed:?} instead of rounding the fortress to {goal:?}"
    );
}

//
// ─── Targeting layers in play ──────────────────────────────────────────────────
//

#[test]
fn melee_ignores_flier_overhead() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // A grunt with a flier parked inside its acquire range. Its axe reaches the
    // ground and the water, so the flier is not a target and the grunt must
    // never start swinging at it.
    let (grunt, _) = utils::create_entity(
        app.world_mut(),
        "grunt",
        utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1),
        Some(0),
    )
    .expect("the grunt spawns");
    let (flier, _) = utils::create_entity(
        app.world_mut(),
        "zeppelin",
        utils::at_cell(CLEAR_GROUND.0 + 1, CLEAR_GROUND.1),
        Some(1),
    )
    .expect("the flier spawns beside it");

    utils::run_ticks(&mut app, 100);

    let world = app.world();
    assert!(
        world
            .entity(grunt)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the grunt took an order against a flier it cannot reach"
    );
    assert_eq!(
        world
            .entity(flier)
            .get::<HealthComponent>()
            .map(|health| health.current()),
        registry_max_health(world, "zeppelin"),
        "the flier took damage from a weapon that cannot reach the air"
    );
}

#[test]
fn archer_engages_flier_overhead() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // The same arrangement with an archer, whose weapon declares every layer:
    // this is the control that proves the test above is about the gate rather
    // than about fliers being unhittable.
    utils::create_entity(
        app.world_mut(),
        "archer",
        utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1),
        Some(0),
    )
    .expect("the archer spawns");
    let (flier, _) = utils::create_entity(
        app.world_mut(),
        "zeppelin",
        utils::at_cell(CLEAR_GROUND.0 + 1, CLEAR_GROUND.1),
        Some(1),
    )
    .expect("the flier spawns beside it");

    utils::run_ticks(&mut app, 100);

    let world = app.world();
    let health = world
        .entity(flier)
        .get::<HealthComponent>()
        .map(|health| health.current());
    // Six arrows of 6 landed over the hundred ticks — the pinned toll of an
    // engagement that genuinely happened, the chasing archer seeing from the
    // cell its body rounds to.
    assert_eq!(
        health,
        Some(FixedU64::from_num(114)),
        "the archer never engaged the flier, so nothing answers one"
    );
}

#[test]
fn garrisoned_melee_ignores_flier_overhead() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // A grunt firing from a bunker: reach is judged by the passenger's own
    // weapon, not the holder's — the bunker has none, and an unarmed holder
    // must not read as reaching everything.
    let (bunker, _) = utils::create_entity(
        app.world_mut(),
        "bunker",
        utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1),
        Some(0),
    )
    .expect("the bunker spawns");
    let (grunt, grunt_id) = utils::create_entity(
        app.world_mut(),
        "grunt",
        utils::at_cell(CLEAR_GROUND.0 + 3, CLEAR_GROUND.1),
        Some(0),
    )
    .expect("the grunt spawns");
    app.world_mut()
        .entity_mut(bunker)
        .get_mut::<TransporterComponent>()
        .expect("the bunker is a holder")
        .passengers
        .insert(grunt_id);
    app.world_mut().entity_mut(grunt).insert(HiddenComponent);

    let (flier, _) = utils::create_entity(
        app.world_mut(),
        "zeppelin",
        utils::at_cell(CLEAR_GROUND.0 + 2, CLEAR_GROUND.1),
        Some(1),
    )
    .expect("the flier parks overhead");

    utils::run_ticks(&mut app, 100);

    let world = app.world();
    assert_eq!(
        world
            .entity(flier)
            .get::<HealthComponent>()
            .map(|health| health.current()),
        registry_max_health(world, "zeppelin"),
        "an axe swung from a bunker reached the air"
    );
}

#[test]
fn garrisoned_archer_engages_flier_overhead() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // The control: the archer's weapon declares the air, so from the same
    // bunker the same flier is fair game.
    let (bunker, _) = utils::create_entity(
        app.world_mut(),
        "bunker",
        utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1),
        Some(0),
    )
    .expect("the bunker spawns");
    let (archer, archer_id) = utils::create_entity(
        app.world_mut(),
        "archer",
        utils::at_cell(CLEAR_GROUND.0 + 3, CLEAR_GROUND.1),
        Some(0),
    )
    .expect("the archer spawns");
    app.world_mut()
        .entity_mut(bunker)
        .get_mut::<TransporterComponent>()
        .expect("the bunker is a holder")
        .passengers
        .insert(archer_id);
    app.world_mut().entity_mut(archer).insert(HiddenComponent);

    let (flier, _) = utils::create_entity(
        app.world_mut(),
        "zeppelin",
        utils::at_cell(CLEAR_GROUND.0 + 2, CLEAR_GROUND.1),
        Some(1),
    )
    .expect("the flier parks overhead");

    utils::run_ticks(&mut app, 100);

    let world = app.world();
    let left = world
        .entity(flier)
        .get::<HealthComponent>()
        .map(|health| health.current());
    // The bunker garrison opened fire later than a standing archer would —
    // the pinned toll says the arrows flew all the same.
    assert_eq!(
        left,
        Some(FixedU64::from_num(132)),
        "the garrisoned archer never fired at the flier: {left:?}"
    );
}

//
// ─── Wide movers ───────────────────────────────────────────────────────────────
//

#[test]
fn wide_flier_crosses_map_and_rests_on_goal() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    let start = (16, 12);
    let goal = (40, 12);
    let (zeppelin, _) = utils::create_entity(
        app.world_mut(),
        "zeppelin",
        utils::at_cell(start.0, start.1),
        Some(1),
    )
    .expect("the zeppelin spawns");
    app.world_mut()
        .entity_mut(zeppelin)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: utils::at_cell(goal.0, goal.1),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    assert!(
        world
            .entity(zeppelin)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the wide flier's walk never finished"
    );
    assert_eq!(
        utils::cell_of(utils::position_of(world, zeppelin)),
        goal,
        "the wide flier did not come to rest on its goal"
    );
}

#[test]
fn wide_flier_claims_its_whole_footprint() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let anchor = (16, 12);
    utils::create_entity(
        app.world_mut(),
        "zeppelin",
        utils::at_cell(anchor.0, anchor.1),
        Some(1),
    )
    .expect("the zeppelin spawns");

    // One tick, so the continuous model rebuilds the claim plane from bodies.
    utils::run_ticks(&mut app, 1);

    let world = app.world();
    let air = world
        .resource::<ContentRegistry>()
        .layer(map::AIR)
        .expect("air layer");
    let live = world.resource::<Map>();
    for dy in 0..2 {
        for dx in 0..2 {
            assert!(
                live.nav_grid()
                    .is_claimed_by(air, CellPos::new(anchor.0 + dx, anchor.1 + dy)),
                "the 2x2 flier does not hold ({}, {})",
                anchor.0 + dx,
                anchor.1 + dy
            );
        }
    }
}

#[test]
fn wide_flier_cannot_spawn_where_its_footprint_does_not_fit() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // The fortress closes a 3x3 of air; a 2x2 anchored one cell short of it
    // still reaches in, so there is nowhere here for the footprint to go.
    utils::create_entity(
        app.world_mut(),
        "sea_fortress",
        utils::at_cell(46, 46),
        None,
    )
    .expect("the fortress stands");

    assert!(
        utils::create_entity(app.world_mut(), "zeppelin", utils::at_cell(45, 46), Some(1))
            .is_none(),
        "a 2x2 flier was placed overlapping the fortress it cannot fit beside"
    );
    assert!(
        utils::create_entity(app.world_mut(), "zeppelin", utils::at_cell(44, 46), Some(1))
            .is_some(),
        "a 2x2 flier could not be placed where its whole footprint is clear"
    );
}

#[test]
fn mixed_fliers_settle_without_milling() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // Two slow wide fliers and two fast ones ordered onto one point. The crowd
    // ladder was written for uniform peers, so this is the check that a mixed
    // group still settles instead of shoving each other around forever; the
    // unequal-size case is engine-tested on the ground.
    let mut units = Vec::new();
    for (type_name, cell) in [
        ("zeppelin", (20, 20)),
        ("zeppelin", (24, 20)),
        ("gryphon_aloft", (20, 24)),
        ("gryphon_aloft", (23, 24)),
    ] {
        let (entity, _) = utils::create_entity(
            app.world_mut(),
            type_name,
            utils::at_cell(cell.0, cell.1),
            Some(1),
        )
        .unwrap_or_else(|| panic!("{type_name} spawns at {cell:?}"));
        units.push(entity);
    }

    let goal = utils::at_cell(30, 30);
    for &unit in &units {
        app.world_mut()
            .entity_mut(unit)
            .get_mut::<OrderQueueComponent>()
            .unwrap()
            .push(
                Order::Move {
                    target: goal,
                    size: CellSize::ONE,
                    range: 0,
                },
                None,
            );
    }

    utils::run_ticks(&mut app, 600);

    for &unit in &units {
        assert!(
            app.world()
                .entity(unit)
                .get::<OrderQueueComponent>()
                .is_some_and(|queue| queue.0.is_empty()),
            "a mixed flight never finished"
        );
    }

    // Settled means still: the mixed pile must come to rest, not churn.
    let settled: Vec<_> = units
        .iter()
        .map(|&unit| utils::position_of(app.world_mut(), unit))
        .collect();
    utils::run_ticks(&mut app, 60);
    let after: Vec<_> = units
        .iter()
        .map(|&unit| utils::position_of(app.world_mut(), unit))
        .collect();
    assert_eq!(
        settled, after,
        "a settled mixed crowd must rest, not mill around"
    );
}

//
// ─── Who shares a cell ─────────────────────────────────────────────────────────
//

#[test]
fn flier_and_walker_share_cell() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let cell = utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1);

    assert!(
        utils::create_entity(app.world_mut(), "peasant", cell, Some(0)).is_some(),
        "the walker spawns on clear ground"
    );
    assert!(
        utils::create_entity(app.world_mut(), "gryphon_aloft", cell, Some(0)).is_some(),
        "the flier was blocked by a walker, but their layers are disjoint"
    );
}

#[test]
fn second_flier_cannot_take_held_air_cell() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let cell = utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1);

    assert!(
        utils::create_entity(app.world_mut(), "gryphon_aloft", cell, Some(0)).is_some(),
        "the first flier spawns"
    );
    assert!(
        utils::create_entity(app.world_mut(), "gryphon_aloft", cell, Some(0)).is_none(),
        "two fliers took the same air cell, so the layer excludes nothing"
    );
}

//
// ─── Death fates aloft ─────────────────────────────────────────────────────────
//

/// Spawns `holder` with a garrisoned `passenger`, both owned by player 0.
fn holder_with_passenger(
    app: &mut App,
    holder: &str,
    passenger: &str,
) -> (Entity, Entity, SimulationId) {
    let (held, _) = utils::create_entity(
        app.world_mut(),
        holder,
        utils::at_cell(CLEAR_GROUND.0, CLEAR_GROUND.1),
        Some(0),
    )
    .unwrap_or_else(|| panic!("'{holder}' spawns"));
    let (aboard, aboard_id) = utils::create_entity(
        app.world_mut(),
        passenger,
        utils::at_cell(CLEAR_GROUND.0 + 3, CLEAR_GROUND.1),
        Some(0),
    )
    .unwrap_or_else(|| panic!("'{passenger}' spawns"));
    app.world_mut()
        .entity_mut(held)
        .get_mut::<TransporterComponent>()
        .expect("the holder carries passengers")
        .passengers
        .insert(aboard_id);
    app.world_mut().entity_mut(aboard).insert(HiddenComponent);
    (held, aboard, aboard_id)
}

#[test]
fn rider_walks_away_from_gryphon_wrecked_on_ground() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, archer, archer_id) = holder_with_passenger(&mut app, "gryphon", "archer");

    spawn::destroy_entity(app.world_mut(), gryphon);
    utils::run_ticks(&mut app, 5);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(archer_id)
            .is_some(),
        "the rider died with a beast cut down on the ground"
    );
    assert!(
        app.world().get::<HiddenComponent>(archer).is_none(),
        "the rider stayed off the map instead of standing beside the wreck"
    );
}

#[test]
fn rider_dies_with_gryphon_shot_from_sky() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _, archer_id) = holder_with_passenger(&mut app, "gryphon_aloft", "archer");

    spawn::destroy_entity(app.world_mut(), gryphon);
    utils::run_ticks(&mut app, 5);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(archer_id)
            .is_none(),
        "the rider walked away from a fall out of the sky"
    );
}

#[test]
fn zeppelin_wreck_takes_passengers_with_it() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (zeppelin, _, peon_id) = holder_with_passenger(&mut app, "zeppelin", "peon");

    spawn::destroy_entity(app.world_mut(), zeppelin);
    utils::run_ticks(&mut app, 5);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(peon_id)
            .is_none(),
        "a passenger walked away from the falling zeppelin"
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// A grass cell near a start point. The demo-map app places nothing, so the
/// only thing that can occupy it is what a test spawns.
const CLEAR_GROUND: (u32, u32) = (16, 12);
/// A river cell: water, and outside both fords, so nothing on foot can stand
/// or cross here.
const RIVER: (u32, u32) = (48, 12);

/// The health a freshly spawned instance of `type_name` starts with.
fn registry_max_health(world: &World, type_name: &str) -> Option<FixedU64> {
    world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .and_then(|def| def.base_stat(EntityStatId::MAX_HEALTH))
}
