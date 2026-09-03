//! Changing form in play: the gryphon walking, taking off, and what the change
//! carries with it.

mod utils;

use ferrets_content::{entity_stats::EntityStatId, registry::ContentRegistry};
use ferrets_demo::map;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        transport::TransporterComponent,
    },
    map::Map,
    movement_model::MovementModel,
    order::Order,
    resources::PlayerResources,
    simulation_id::SimulationId,
    spawn,
};

//
// ─── The change itself ─────────────────────────────────────────────────────────
//

#[test]
fn taking_off_swaps_occupied_layer() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    command_morph(&mut app, gryphon_id, "gryphon_aloft");
    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft");

    let world = app.world();
    let registry = world.resource::<ContentRegistry>();
    let ground = registry.layer(map::GROUND).unwrap();
    let air = registry.layer(map::AIR).unwrap();
    let live = world.resource::<Map>();
    for dy in 0..2 {
        for dx in 0..2 {
            let cell = CellPos::new(CLEAR.0 + dx, CLEAR.1 + dy);
            assert!(
                live.nav_grid().is_passable(ground, cell),
                "the airborne form still holds the ground at {cell:?}"
            );
            assert!(
                !live.nav_grid().is_passable(air, cell),
                "the airborne form does not hold the air at {cell:?}"
            );
        }
    }
}

#[test]
fn landing_is_refused_when_ground_is_taken() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon_aloft",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the airborne gryphon spawns");

    // A farm under it takes the ground its landing form would need: nothing
    // may land on an occupied spot, and what stands there is not crushed to
    // make room.
    spawn::create_entity(
        app.world_mut(),
        "farm",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the farm is raised under it");

    // The landing reserves its ground the moment the order starts, so taken
    // ground refuses the order on the spot and the change never begins.
    command_morph(&mut app, gryphon_id, "gryphon");
    assert_eq!(
        type_name_of(&app, gryphon),
        "gryphon_aloft",
        "a refused landing must leave the unit as it was"
    );
    // And it must still hold its own air, not have been quietly displaced.
    let world = app.world();
    let air = world.resource::<ContentRegistry>().layer(map::AIR).unwrap();
    assert!(
        !world
            .resource::<Map>()
            .nav_grid()
            .is_passable(air, CellPos::new(CLEAR.0, CLEAR.1)),
        "a refused landing left the unit off the map"
    );
}

#[test]
fn changing_form_is_refused_when_riders_would_not_fit() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    // Two riders crammed aboard a one-seat holder: its other form has one seat
    // too, so the change is refused rather than spilling anyone. The overload
    // itself is staged directly — boarding would never have admitted a second.
    for offset in [3, 4] {
        let (_, rider) = spawn::create_entity(
            app.world_mut(),
            "archer",
            utils::at_cell(CLEAR.0 + offset, CLEAR.1),
            Some(0),
        )
        .expect("a rider spawns");
        app.world_mut()
            .entity_mut(gryphon)
            .get_mut::<TransporterComponent>()
            .expect("the gryphon is a holder")
            .passengers
            .insert(rider);
    }

    command_morph(&mut app, gryphon_id, "gryphon_aloft");
    assert_eq!(
        type_name_of(&app, gryphon),
        "gryphon",
        "an overloaded change went through instead of being refused"
    );
}

#[test]
fn changing_into_anything_but_declared_form_is_refused() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // Even a type that has transitions may only become what they name; the
    // type-with-no-transitions case is the command-gate test below. Without
    // this gate any wire-legal command could turn any unit into any
    // registered type.
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    command_morph(&mut app, gryphon_id, "zeppelin");
    assert_eq!(
        type_name_of(&app, gryphon),
        "gryphon",
        "a form changed into something its type declares no transition into"
    );
}

//
// ─── What the change carries ───────────────────────────────────────────────────
//

#[test]
fn health_carries_its_proportion_not_its_amount() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    let max = app
        .world()
        .resource::<ContentRegistry>()
        .entity("gryphon")
        .and_then(|def| def.base_stat(EntityStatId::MAX_HEALTH))
        .expect("the gryphon has health");
    // Halve it, then change form: the fraction is what must survive.
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<HealthComponent>()
        .unwrap()
        .apply_damage(max / 2);

    command_morph(&mut app, gryphon_id, "gryphon_aloft");

    let after = app
        .world()
        .entity(gryphon)
        .get::<HealthComponent>()
        .unwrap()
        .current();
    let aloft_max = app
        .world()
        .resource::<ContentRegistry>()
        .entity("gryphon_aloft")
        .and_then(|def| def.base_stat(EntityStatId::MAX_HEALTH))
        .expect("the airborne form has health");
    assert_eq!(after, aloft_max / 2);
}

#[test]
fn order_queue_survives_form_change() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    // Sent aloft with a walk shift-queued behind the change: it still wants
    // to go there, and it is the new form's business how to get there.
    utils::select(&mut app, gryphon_id, SelectMode::Replace);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "gryphon_aloft".to_string(),
            flush: true,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::at_cell(CLEAR.0 + 8, CLEAR.1),
            flush: false,
        },
    );
    // Just past the change's window: the walk has barely started, so it must
    // still be in the queue of the new form.
    utils::run_ticks(&mut app, utils::APPLY + 22);

    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft");
    assert!(
        !app.world()
            .entity(gryphon)
            .get::<OrderQueueComponent>()
            .expect("the queue survives")
            .0
            .is_empty(),
        "the walk was thrown away with the old form"
    );
}

//
// ─── The timed order ───────────────────────────────────────────────────────────
//

#[test]
fn ordered_change_takes_forms_morph_time() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: "gryphon_aloft".to_string(),
            },
            None,
        );

    // Part-way through the window it is still on the ground: the change is a
    // commitment with a duration, not an instant flip.
    utils::run_ticks(&mut app, 10);
    assert_eq!(type_name_of(&app, gryphon), "gryphon");

    utils::run_ticks(&mut app, 20);
    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft");
    assert!(
        app.world()
            .entity(gryphon)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the change finished but its order did not"
    );
}

#[test]
fn changed_gryphon_flies_over_what_stopped_it_on_foot() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // On the near bank of the river, ordered across it. On foot the fordless
    // stretch is a wall; aloft it is nothing.
    let (gryphon, _) =
        spawn::create_entity(app.world_mut(), "gryphon", utils::at_cell(44, 12), Some(0))
            .expect("the gryphon spawns by the river");
    let goal = (52, 12);

    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: "gryphon_aloft".to_string(),
            },
            None,
        );
    utils::run_ticks(&mut app, 30);
    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft");

    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: utils::at_cell(goal.0, goal.1),
                size: ferrets_geometry::cell_size::CellSize::ONE,
                range: 0,
            },
            None,
        );
    utils::run_ticks(&mut app, 400);

    assert_eq!(
        utils::cell_of(utils::position_of(app.world_mut(), gryphon)),
        goal,
        "the airborne gryphon did not cross the river"
    );
}

//
// ─── Fighting from the saddle ──────────────────────────────────────────────────
//

#[test]
fn rider_fires_from_moving_holder() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    // An airborne gryphon with an archer aboard, and an enemy grunt to shoot at.
    // The holder is what stands on the map, so every range and fog reading is
    // its — and it is moving, so firing must not depend on the holder standing
    // still.
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon_aloft",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    let (rider, rider_id) = spawn::create_entity(
        app.world_mut(),
        "archer",
        utils::at_cell(CLEAR.0 + 4, CLEAR.1),
        Some(0),
    )
    .expect("the rider spawns");
    // Aboard: hidden, holding no cells, taking no orders of its own.
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<TransporterComponent>()
        .expect("the gryphon is a holder")
        .passengers
        .insert(rider_id);
    app.world_mut()
        .entity_mut(rider)
        .insert(ferrets_simulation::components::hidden::HiddenComponent);

    let (_, victim_id) = spawn::create_entity(
        app.world_mut(),
        "grunt",
        utils::at_cell(CLEAR.0 + 3, CLEAR.1 + 1),
        Some(1),
    )
    .expect("the victim spawns");

    // Send the holder somewhere while its rider works: firing must not depend on
    // the holder standing still.
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: utils::at_cell(CLEAR.0 + 6, CLEAR.1 + 2),
                size: ferrets_geometry::cell_size::CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 150);

    // Killed outright or merely mauled, either way the rider worked; a victim at
    // full health would mean it never fired at all.
    let full = app
        .world()
        .resource::<ContentRegistry>()
        .entity("grunt")
        .and_then(|def| def.base_stat(EntityStatId::MAX_HEALTH))
        .expect("the grunt has health");
    let outcome = app
        .world()
        .resource::<ferrets_simulation::entity_index::EntityIndex>()
        .alive(victim_id)
        .map(|victim| {
            app.world()
                .entity(victim)
                .get::<HealthComponent>()
                .map(|health| health.current())
        });
    match outcome {
        None => {}
        Some(left) => assert!(
            left.is_some_and(|left| left < full),
            "the rider never fired from the moving holder"
        ),
    }
}

//
// ─── Determinism ───────────────────────────────────────────────────────────────
//

/// One run of the whole feature — wide fliers, a crowd, and a form change
/// mid-game — sampled as a checksum every tick.
fn run_and_checksum() -> Vec<u64> {
    let mut app = utils::demo_map_app(MovementModel::Continuous);

    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    for (type_name, cell) in [
        ("zeppelin", (24, 20)),
        ("zeppelin", (20, 24)),
        ("grunt", (22, 18)),
    ] {
        spawn::create_entity(
            app.world_mut(),
            type_name,
            utils::at_cell(cell.0, cell.1),
            Some(1),
        )
        .unwrap_or_else(|| panic!("{type_name} spawns"));
    }

    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: "gryphon_aloft".to_string(),
            },
            None,
        );
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: utils::at_cell(30, 26),
                size: ferrets_geometry::cell_size::CellSize::ONE,
                range: 0,
            },
            None,
        );

    let mut checksums = Vec::new();
    for _ in 0..200 {
        utils::run_ticks(&mut app, 1);
        checksums.push(ferrets_simulation::checksum::state_checksum(app.world()));
    }
    checksums
}

#[test]
fn form_change_is_deterministic() {
    // The same inputs must produce the same state every tick. Type identity is
    // folded into the checksum, so a form change landing on a different tick — or
    // not at all — shows up here rather than surfacing later as a desync.
    assert_eq!(run_and_checksum(), run_and_checksum());
}

//
// ─── Death and instant changes ─────────────────────────────────────────────────
//

#[test]
fn death_flushes_change_under_way() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: "gryphon_aloft".to_string(),
            },
            None,
        );
    utils::run_ticks(&mut app, 5);
    assert_eq!(type_name_of(&app, gryphon), "gryphon", "still changing");

    // Killed mid-window: dying overrides the commitment. Without the force
    // cancel the corpse would stand through the remaining ticks, change form,
    // and only then die.
    spawn::destroy_entity(app.world_mut(), gryphon);
    utils::run_ticks(&mut app, 8);

    assert!(
        app.world()
            .resource::<ferrets_simulation::entity_index::EntityIndex>()
            .alive(id)
            .is_none(),
        "a unit killed mid-change outlived its dying phase"
    );
}

#[test]
fn quickened_change_lands_same_tick() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    // The window is the entity's own effective stat, so play can move it: a
    // change quickened to zero waits for nothing. The stat is content's own,
    // so its id is looked up rather than named.
    let morph_time = app
        .world()
        .resource::<ContentRegistry>()
        .entity_stat("morph_time")
        .expect("demo declares its morph_time stat");
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<ferrets_simulation::components::entity_stats::StatsComponent>()
        .unwrap()
        .set_base(morph_time, ferrets_math::FixedU64::ZERO);

    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: "gryphon_aloft".to_string(),
            },
            None,
        );
    utils::run_ticks(&mut app, 1);

    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft");
}

//
// ─── The command path ──────────────────────────────────────────────────────────
//

#[test]
fn morph_command_gates_on_declared_transitions() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    // The executor is the wire boundary: a command may name any registered
    // type, and only what the selected entity's own type declares may queue.
    let (peasant, peasant_id) = spawn::create_entity(
        app.world_mut(),
        "peasant",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the peasant spawns");

    utils::select(&mut app, peasant_id, SelectMode::Replace);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "sea_fortress".to_string(),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);

    assert_eq!(type_name_of(&app, peasant), "peasant");
    assert!(
        app.world()
            .entity(peasant)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "an undeclared change took the queue"
    );
}

#[test]
fn morph_command_changes_everyone_that_can() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    // A mixed selection: the gryphon declares the transition, the peasant
    // does not — the executor changes whoever can and drops the rest.
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    let (peasant, peasant_id) = spawn::create_entity(
        app.world_mut(),
        "peasant",
        utils::at_cell(CLEAR.0 + 4, CLEAR.1),
        Some(0),
    )
    .expect("the peasant spawns");

    utils::select(&mut app, gryphon_id, SelectMode::Replace);
    utils::select(&mut app, peasant_id, SelectMode::Add);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "gryphon_aloft".to_string(),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 30);

    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft");
    assert_eq!(type_name_of(&app, peasant), "peasant");
}

//
// ─── The transition's terms ────────────────────────────────────────────────────
//

/// Pushes a Morph order into `type_name` onto the entity's queue.
fn order_morph(app: &mut bevy::prelude::App, entity: bevy::prelude::Entity, type_name: &str) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: type_name.to_string(),
            },
            None,
        );
}

/// The entity's current energy.
fn energy_of(app: &bevy::prelude::App, entity: bevy::prelude::Entity) -> FixedU64 {
    app.world()
        .entity(entity)
        .get::<EnergyComponent>()
        .expect("the entity has an energy pool")
        .current()
}

#[test]
fn take_off_draws_its_energy_cost() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    assert_eq!(energy_of(&app, gryphon), FixedU64::from_num(60));

    // The cost is drawn when the order starts, not when the change lands: the
    // wing-beat is spent on the spot, well before the window runs out.
    order_morph(&mut app, gryphon, "gryphon_aloft");
    utils::run_ticks(&mut app, 2);
    assert_eq!(type_name_of(&app, gryphon), "gryphon", "still changing");
    // The wing-beat's 20 off the full 60 pool, plus two ticks of the 0.2
    // regen in binary fixed-point.
    assert_eq!(
        energy_of(&app, gryphon),
        FixedU64::from_bits(0x28_6666_6666),
        "the take-off never drew its energy cost"
    );
}

#[test]
fn unpayable_cost_refuses_change() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<EnergyComponent>()
        .unwrap()
        .spend(FixedU64::from_num(55));

    // Five energy against a cost of twenty: the order is refused whole rather
    // than started on credit — and nothing is drawn from the pool.
    order_morph(&mut app, gryphon, "gryphon_aloft");
    utils::run_ticks(&mut app, 5);
    assert_eq!(type_name_of(&app, gryphon), "gryphon");
    assert!(
        app.world()
            .entity(gryphon)
            .get::<OrderQueueComponent>()
            .unwrap()
            .0
            .is_empty(),
        "an unpayable change squatted in the queue"
    );
}

#[test]
fn landing_reserves_its_ground_for_whole_window() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon_aloft",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the airborne gryphon spawns");

    order_morph(&mut app, gryphon, "gryphon");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft", "still aloft");

    // Mid-window the ground below is already spoken for: a farm raised on the
    // spot must be refused, or the descent it was promised would fizzle.
    assert!(
        spawn::create_entity(
            app.world_mut(),
            "farm",
            utils::at_cell(CLEAR.0, CLEAR.1),
            Some(0),
        )
        .is_none(),
        "a farm was raised on ground a landing reserved"
    );

    utils::run_ticks(&mut app, 25);
    assert_eq!(
        type_name_of(&app, gryphon),
        "gryphon",
        "the reserved landing did not complete"
    );
    // Landed, the reservation is spent: the same spot refuses a farm now for
    // the plain reason that the gryphon is standing on it.
}

#[test]
fn reservation_holds_under_cell_model() {
    // Under the cell model claims are law and nothing rebuilds them each tick,
    // so the reservation written at the order's start is exactly what must
    // still be standing when the change lands.
    let mut app = utils::demo_map_app(MovementModel::Cell);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon_aloft",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the airborne gryphon spawns");

    order_morph(&mut app, gryphon, "gryphon");
    utils::run_ticks(&mut app, 3);
    assert!(
        spawn::create_entity(
            app.world_mut(),
            "farm",
            utils::at_cell(CLEAR.0, CLEAR.1),
            Some(0),
        )
        .is_none(),
        "a farm was raised on ground a landing reserved"
    );

    utils::run_ticks(&mut app, 25);
    assert_eq!(type_name_of(&app, gryphon), "gryphon");
}

// The same refusal as landing_is_refused_when_ground_is_taken, driven through
// the raw order rather than the command path: this one pins the queue-level
// outcome (the entry finishes at once), where the command test pins the
// standing state the player sees.
#[test]
fn landing_order_is_refused_when_ground_is_taken_at_start() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon_aloft",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the airborne gryphon spawns");
    spawn::create_entity(
        app.world_mut(),
        "farm",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the farm is raised under it");

    // A reserving transition refuses on the spot when the ground is taken,
    // instead of descending for a second and fizzling where a player is not
    // looking.
    order_morph(&mut app, gryphon, "gryphon");
    utils::run_ticks(&mut app, 25);
    assert_eq!(type_name_of(&app, gryphon), "gryphon_aloft");
    assert!(
        app.world()
            .entity(gryphon)
            .get::<OrderQueueComponent>()
            .unwrap()
            .0
            .is_empty(),
        "a hopeless landing squatted in the queue"
    );
}

#[test]
fn contested_take_off_fizzles_and_keeps_payment() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    // Taking off revalidates: nothing is claimed early, so the sky above can be
    // taken while the beast is still spreading its wings.
    order_morph(&mut app, gryphon, "gryphon_aloft");
    utils::run_ticks(&mut app, 5);
    spawn::create_entity(
        app.world_mut(),
        "zeppelin",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the zeppelin parks in its sky");

    utils::run_ticks(&mut app, 30);
    assert_eq!(
        type_name_of(&app, gryphon),
        "gryphon",
        "it took off into an occupied sky"
    );
    // A committed transition keeps the payment when the change fizzles: the
    // pool shows the drain (minus what regenerated meanwhile), not a refund.
    // The drain minus 35 ticks of the 0.2 regen in binary fixed-point —
    // just short of 47, nowhere near a refunded 60.
    assert_eq!(
        energy_of(&app, gryphon),
        FixedU64::from_bits(0x2e_ffff_fff9),
        "a committed transition's cost came back"
    );
}

#[test]
fn upgrade_pays_up_front_and_lands() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (tower, _) = spawn::create_entity(
        app.world_mut(),
        "watch_tower",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(1),
    )
    .expect("the tower spawns");
    {
        let mut stock = app.world_mut().resource_mut::<PlayerResources>();
        stock.add(1, "gold", 100);
        stock.add(1, "wood", 30);
    }

    order_morph(&mut app, tower, "guard_tower");
    utils::run_ticks(&mut app, 2);
    let stock = app.world().resource::<PlayerResources>();
    assert_eq!(stock.amount(1, "gold"), 20, "the gold was not committed");
    assert_eq!(stock.amount(1, "wood"), 10, "the wood was not committed");
    assert_eq!(type_name_of(&app, tower), "watch_tower", "still upgrading");

    utils::run_ticks(&mut app, 65);
    assert_eq!(type_name_of(&app, tower), "guard_tower");
}

#[test]
fn cancelled_upgrade_returns_its_money() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (tower, _) = spawn::create_entity(
        app.world_mut(),
        "watch_tower",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(1),
    )
    .expect("the tower spawns");
    {
        let mut stock = app.world_mut().resource_mut::<PlayerResources>();
        stock.add(1, "gold", 100);
        stock.add(1, "wood", 30);
    }

    order_morph(&mut app, tower, "guard_tower");
    utils::run_ticks(&mut app, 10);
    assert_eq!(
        app.world().resource::<PlayerResources>().amount(1, "gold"),
        20,
        "the upgrade never started"
    );

    // A refundable transition honors a plain cancel and gives the full cost
    // back — which is what makes starting one cheap to reconsider.
    app.world_mut()
        .entity_mut(tower)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .cancel_all(CancelPolicy::Soft);
    utils::run_ticks(&mut app, 2);

    assert_eq!(type_name_of(&app, tower), "watch_tower");
    let stock = app.world().resource::<PlayerResources>();
    assert_eq!(stock.amount(1, "gold"), 100, "the gold did not come back");
    assert_eq!(stock.amount(1, "wood"), 30, "the wood did not come back");
    utils::run_ticks(&mut app, 60);
    assert_eq!(
        type_name_of(&app, tower),
        "watch_tower",
        "a cancelled upgrade landed anyway"
    );
}

#[test]
fn committed_change_shrugs_off_cancel() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, _) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");

    order_morph(&mut app, gryphon, "gryphon_aloft");
    utils::run_ticks(&mut app, 5);
    app.world_mut()
        .entity_mut(gryphon)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .cancel_all(CancelPolicy::Soft);
    utils::run_ticks(&mut app, 25);

    assert_eq!(
        type_name_of(&app, gryphon),
        "gryphon_aloft",
        "a committed change was talked out of itself"
    );
}

#[test]
fn melee_breaks_off_when_its_target_takes_flight() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (gryphon, gryphon_id) = spawn::create_entity(
        app.world_mut(),
        "gryphon",
        utils::at_cell(CLEAR.0, CLEAR.1),
        Some(0),
    )
    .expect("the gryphon spawns");
    // An unarmed mover defaults to fleeing when hit, which would end the fight
    // by escape; standing its ground keeps the test about reachability.
    app.world_mut()
        .entity_mut(gryphon)
        .remove::<ferrets_simulation::components::stance::StanceComponent>();
    let (grunt, _) = spawn::create_entity(
        app.world_mut(),
        "grunt",
        utils::at_cell(CLEAR.0 + 2, CLEAR.1),
        Some(1),
    )
    .expect("the grunt spawns");

    // An axe legally raised against the grounded form. Once the target is
    // airborne the weapon's layers no longer reach it, so the order must
    // finish rather than chase and swing at something no hit can land on.
    app.world_mut()
        .entity_mut(grunt)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Attack {
                target: ferrets_simulation::order::AttackTarget::Entity(gryphon_id),
                leash: None,
            },
            None,
        );
    utils::run_ticks(&mut app, 10);
    assert!(
        !app.world()
            .entity(grunt)
            .get::<OrderQueueComponent>()
            .unwrap()
            .0
            .is_empty(),
        "the grounded fight never started"
    );

    command_morph(&mut app, gryphon_id, "gryphon_aloft");
    utils::run_ticks(&mut app, 20);

    assert!(
        app.world()
            .entity(grunt)
            .get::<OrderQueueComponent>()
            .unwrap()
            .0
            .is_empty(),
        "the axe kept swinging at a target it can no longer reach"
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Clear grass well away from anything the map places.
const CLEAR: (u32, u32) = (16, 12);

/// Sends a Morph command for the entity through the executor — the same wire
/// the HUD uses — and runs the change's whole window out.
fn command_morph(app: &mut bevy::prelude::App, id: SimulationId, type_name: &str) {
    utils::select(app, id, SelectMode::Replace);
    utils::push_command(
        app,
        PlayerCommand::Morph {
            type_name: type_name.to_string(),
            flush: true,
        },
    );
    utils::run_ticks(app, utils::APPLY + 25);
}

/// The type an entity currently is.
fn type_name_of(app: &bevy::prelude::App, entity: bevy::prelude::Entity) -> String {
    app.world()
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("a live entity carries its info")
        .type_name()
        .to_string()
}
