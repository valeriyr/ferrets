//! Remains: which deaths leave a body, what a body remembers, where it is set
//! down, and how many the map holds.

use bevy::prelude::*;
use ferrets_content::{
    affiliation::Affiliation,
    attack::Slain,
    dying::{Bequest, DeathKind, LeftBy},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    kinds::Kinds,
    location::Solidity,
    registry::ContentRegistry,
    transport::{PassengerConduct, PassengerFate},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        dying::RemainsComponent, entity_info::EntityInfoComponent, hidden::HiddenComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::DeathCause,
    game_loop::damage,
    ruleset::{RemainsLimit, Ruleset},
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn,
};

mod utils;

//
// ─── Which deaths leave a body ──────────────────────────────────────────────
//

#[test]
fn killed_entity_leaves_body() {
    let mut app = app(RemainsLimit::Unbounded);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 7, 5, 1);

    kill(&mut app, soldier, killer, Slain::Remains);

    assert_eq!(bodies(&app).len(), 1, "a soldier killed in the field falls");
}

#[test]
fn body_remembers_type_and_owner_that_fell() {
    let mut app = app(RemainsLimit::Unbounded);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 7, 5, 1);
    let soldier_type = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("soldier")
        .expect("the type is registered");

    kill(&mut app, soldier, killer, Slain::Remains);

    let body = bodies(&app)[0];
    let remembered = app
        .world()
        .entity(body)
        .get::<RemainsComponent>()
        .expect("remains carry what fell there");
    assert_eq!(
        remembered.of, soldier_type,
        "a body remembers the type that fell, not the type it is"
    );
    assert_eq!(remembered.owner, Some(0), "and whose it was");
}

#[test]
fn weapon_that_leaves_nothing_leaves_no_body() {
    let mut app = app(RemainsLimit::Unbounded);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 7, 5, 1);

    kill(&mut app, soldier, killer, Slain::Nothing);

    assert!(
        bodies(&app).is_empty(),
        "a shelled body is not a body anyone can raise"
    );
}

#[test]
fn death_type_does_not_name_leaves_no_body() {
    let mut app = app(RemainsLimit::Unbounded);
    // The sapper names only `killed`, so withering away leaves nothing.
    let (sapper, _) = utils::create_owned(&mut app, "sapper", 5, 5, 0);

    spawn::despawn_entity(app.world_mut(), sapper, DeathCause::Decayed);
    utils::run_ticks(&mut app, 4);

    assert!(
        bodies(&app).is_empty(),
        "a death the type does not name leaves nothing"
    );
}

#[test]
fn death_type_names_leaves_body() {
    let mut app = app(RemainsLimit::Unbounded);
    let (sapper, _) = utils::create_owned(&mut app, "sapper", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 7, 5, 1);

    kill(&mut app, sapper, killer, Slain::Remains);

    assert_eq!(
        bodies(&app).len(),
        1,
        "a death the type names by name leaves what it names"
    );
}

#[test]
fn type_that_names_no_deaths_leaves_body() {
    let mut app = app(RemainsLimit::Unbounded);
    // The soldier leaves what the engine's own rule leaves, so withering away
    // — a death that ends a life rather than taking it off the board — falls.
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);

    spawn::despawn_entity(app.world_mut(), soldier, DeathCause::Decayed);
    utils::run_ticks(&mut app, 4);

    assert_eq!(
        bodies(&app).len(),
        1,
        "a type naming no deaths leaves a body for every death that ends a life"
    );
}

#[test]
fn canceled_construction_leaves_no_body_under_its_own_rule() {
    let mut app = app(RemainsLimit::Unbounded);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);

    spawn::despawn_entity(app.world_mut(), soldier, DeathCause::Canceled);
    utils::run_ticks(&mut app, 4);

    assert!(
        bodies(&app).is_empty(),
        "what is taken off the board leaves nothing, however it falls"
    );
}

//
// ─── Where a body is set down ───────────────────────────────────────────────
//

#[test]
fn body_is_set_down_under_standing_mover() {
    let mut app = app(RemainsLimit::Unbounded);
    // A wraith claims no cells, so a soldier stands in the same cell it dies
    // in — and the body it leaves claims none either.
    let (wraith, _) = utils::create_owned(&mut app, "wraith", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 5, 5, 1);

    kill(&mut app, wraith, killer, Slain::Remains);

    let left = bodies(&app);
    assert_eq!(
        left.len(),
        1,
        "a body that claims no cells is set down whoever stands over it"
    );
    assert_eq!(
        utils::cell_of(app.world(), left[0]),
        CellPos::new(5, 5),
        "and it lies where the wraith fell, not beside the soldier standing there"
    );
}

#[test]
fn solid_remains_are_not_laid_over_burrowed_entity() {
    let mut app = app(RemainsLimit::Unbounded);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 12, 12, 1);
    // The mole claims nothing on the grid, so the cart stands on its cell; the
    // rubble the cart leaves would hold that ground, and is refused as a site
    // founded there is.
    utils::create_owned(&mut app, "mole", 5, 5, 0);
    let (cart, _) = utils::create_owned(&mut app, "cart", 5, 5, 0);

    kill(&mut app, cart, killer, Slain::Remains);
    assert!(
        bodies(&app).is_empty(),
        "no rubble is laid over what has dug in"
    );

    // On open ground the same death lays it.
    let (cart, _) = utils::create_owned(&mut app, "cart", 8, 5, 0);
    kill(&mut app, cart, killer, Slain::Remains);
    let left = bodies(&app);
    assert_eq!(left.len(), 1);
    assert_eq!(utils::cell_of(app.world(), left[0]), CellPos::new(8, 5));
}

//
// ─── How many the map holds ─────────────────────────────────────────────────
//

#[test]
fn death_beyond_remains_limit_leaves_no_body() {
    let mut app = app(RemainsLimit::AtMost(2));
    let (_, killer) = utils::create_owned(&mut app, "soldier", 9, 9, 1);

    for x in [3, 4, 5] {
        let (soldier, _) = utils::create_owned(&mut app, "soldier", x, 5, 0);
        kill(&mut app, soldier, killer, Slain::Remains);
    }

    let lying: Vec<CellPos> = bodies(&app)
        .into_iter()
        .map(|body| utils::cell_of(app.world(), body))
        .collect();
    assert_eq!(
        lying,
        vec![CellPos::new(3, 5), CellPos::new(4, 5)],
        "the two the rules allow lie where the first two fell, and the third \
         death left nothing: what lies there stays, and decay makes the room"
    );
}

#[test]
fn body_past_its_decay_is_not_thinned_for_what_it_leaves() {
    let mut app = app(RemainsLimit::AtMost(1));
    let (_, killer) = utils::create_owned(&mut app, "soldier", 9, 9, 1);
    let (ghoul, _) = utils::create_owned(&mut app, "ghoul", 5, 5, 0);

    // The ash lies alone under a cap of one. Its own decay is 3 ticks, and
    // what it rots into asks the cap for room on the tick it ends: a body
    // already leaving is no longer one the cap may take, or it would be
    // despawned out from under the death still handing on what it leaves.
    kill(&mut app, ghoul, killer, Slain::Remains);
    utils::run_ticks(&mut app, 6);

    let left = bodies(&app);
    assert_eq!(
        left.len(),
        1,
        "one body lies there, the cap's whole allowance"
    );
    assert_eq!(
        entity_def::type_name(app.world(), left[0]),
        "dust",
        "and it is what the ash rotted into, the ash itself being gone"
    );
}

#[test]
fn unbounded_remains_hold_every_body() {
    let mut app = app(RemainsLimit::Unbounded);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 9, 9, 1);

    for x in [3, 4, 5] {
        let (soldier, _) = utils::create_owned(&mut app, "soldier", x, 5, 0);
        kill(&mut app, soldier, killer, Slain::Remains);
    }

    assert_eq!(
        bodies(&app).len(),
        3,
        "rules that cap nothing leave every body where it fell"
    );
}

//
// ─── Units a death hands on ─────────────────────────────────────────────────
//

#[test]
fn kill_stands_up_what_is_not_body() {
    let mut app = app(RemainsLimit::Unbounded);
    let (hive, _) = utils::create_owned(&mut app, "hive", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 12, 12, 1);

    kill(&mut app, hive, killer, Slain::Remains);

    let brood = standing(&app, "broodling");
    assert_eq!(brood.len(), 2, "both of what was growing inside spill out");
    assert!(
        brood
            .iter()
            .all(|&entity| entity_def::owner(app.world(), entity) == Some(0)),
        "and they belong to whoever owned the hive"
    );
    // The hive stood on (5, 5) across three cells, so its middle is (6, 6):
    // the search starts on the ground it freed and offers the middle first,
    // then the cell above it — so what bursts out stands about the middle of
    // the ruin rather than in the corner a row-major scan would start from.
    let cells: Vec<CellPos> = brood
        .iter()
        .map(|&entity| CellPos::from(entity_def::position(app.world(), entity)))
        .collect();
    assert_eq!(
        cells,
        vec![CellPos::new(6, 6), CellPos::new(6, 5)],
        "on the ground the hive was standing on, about its middle"
    );
}

#[test]
fn body_of_wide_building_lies_about_its_middle() {
    let mut app = app(RemainsLimit::Unbounded);
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 12, 12, 1);

    kill(&mut app, keep, killer, Slain::Remains);

    let bodies = bodies(&app);
    let [body] = bodies.as_slice() else {
        panic!("the keep left exactly one body");
    };
    // Three cells from (5, 5), so the middle is (6, 6): a body shares the
    // ground it is laid on, so it takes that middle rather than the corner.
    assert_eq!(
        CellPos::from(entity_def::position(app.world(), *body)),
        CellPos::new(6, 6),
        "a body lies where the building's middle was"
    );
}

#[test]
fn body_decaying_makes_room_under_limit() {
    let mut app = app(RemainsLimit::AtMost(1));
    let (_, killer) = utils::create_owned(&mut app, "soldier", 12, 12, 1);
    // The imp leaves a cinder, which lies thirty ticks and rots into nothing.
    let (imp, _) = utils::create_owned(&mut app, "imp", 4, 4, 0);
    kill(&mut app, imp, killer, Slain::Remains);
    assert_eq!(bodies(&app).len(), 1, "the cinder is the one body allowed");

    // A second death while the cinder lies: the cap refuses the new body.
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 8, 4, 0);
    kill(&mut app, soldier, killer, Slain::Remains);
    assert_eq!(
        bodies(&app).len(),
        1,
        "no room for a second while the first lies"
    );

    // The cinder's thirty ticks run out — four of them spent by the kill
    // above — and the slot it held is free for the next death.
    utils::run_ticks(&mut app, 30);
    assert!(bodies(&app).is_empty(), "the cinder rotted away");
    let (another, _) = utils::create_owned(&mut app, "soldier", 9, 4, 0);
    kill(&mut app, another, killer, Slain::Remains);

    let bodies = bodies(&app);
    let [body] = bodies.as_slice() else {
        panic!("the slot the decay freed holds exactly one body");
    };
    assert_eq!(
        app.world()
            .entity(*body)
            .get::<EntityInfoComponent>()
            .expect("a body is an entity like any other")
            .type_name(),
        "corpse",
        "and what lies in it is the new body, not what rotted"
    );
}

#[test]
fn passenger_that_goes_down_with_its_carrier_leaves_nothing() {
    let mut app = app(RemainsLimit::Unbounded);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 14, 14, 1);
    let (_, wagon) = utils::create_owned(&mut app, "wagon", 8, 8, 0);
    let (_, rider) = utils::create_owned(&mut app, "rider", 9, 8, 0);

    // Aboard, and taken down with the wagon: the rider was off the map, so
    // there was no ground under it to leave a body on — least of all the cell
    // it happened to step aboard from.
    utils::select(&mut app, rider);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: wagon,
            flush: true,
        },
    );
    // Three ticks carry the command in, and the fourth boards: the rider
    // stands one cell from the wagon, inside its load range of two, so the
    // order arrives on its first work tick and the wagon admits it there.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let carrier = app
        .world()
        .resource::<EntityIndex>()
        .alive(wagon)
        .expect("the wagon is standing");
    let aboard = app
        .world()
        .resource::<EntityIndex>()
        .alive(rider)
        .expect("the rider is still alive");
    assert!(
        app.world().entity(aboard).contains::<HiddenComponent>(),
        "the rider is aboard, and so off the map"
    );

    kill(&mut app, carrier, killer, Slain::Remains);

    assert!(
        bodies(&app).is_empty(),
        "a death off the map hands nothing on, though the rider's type names that very death"
    );
}

#[test]
fn weapon_that_leaves_no_body_still_lets_rest_out() {
    let mut app = app(RemainsLimit::Unbounded);
    let (hive, _) = utils::create_owned(&mut app, "hive", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "soldier", 12, 12, 1);

    kill(&mut app, hive, killer, Slain::Nothing);

    assert_eq!(
        standing(&app, "broodling").len(),
        2,
        "a shell denies a body, not what bursts out of the thing it kills"
    );
}

#[test]
fn death_type_does_not_name_leaves_none_of_it() {
    let mut app = app(RemainsLimit::Unbounded);
    let (hive, _) = utils::create_owned(&mut app, "hive", 5, 5, 0);

    // The hive's brood names `killed` only: a hive that withers leaves nothing.
    spawn::despawn_entity(app.world_mut(), hive, DeathCause::Decayed);
    utils::run_ticks(&mut app, 4);

    assert!(standing(&app, "broodling").is_empty());
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// App with two players, a `corpse` type, a `soldier` leaving one by the
/// engine's own rule, a `wraith` that claims no cells, a `sapper` leaving one
/// only when something kills it, a `hive` that leaves two `broodling`s
/// standing when something kills it, a `cart` leaving `rubble` that claims its
/// cell, and a `mole` that stands underfoot.
fn app(limit: RemainsLimit) -> App {
    let mut app = utils::make_app_under(
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::occupied(1, PlayerType::Human, None, None),
        ],
        Ruleset::new(limit),
    );
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();

        registry.register(
            EntityTypeDef::new("corpse")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
        );
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(30)
                .with_attack(utils::weapon(utils::GROUND), 10, 1, 1, 4, 2)
                .with_dying(2, utils::leaves("corpse")),
        );
        registry.register(
            EntityTypeDef::new("dust")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
        );
        registry.register(
            EntityTypeDef::new("ash")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(3))
                .with_leaves(utils::leaves("dust")),
        );
        registry.register(
            EntityTypeDef::new("ghoul")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(30)
                .with_dying(2, utils::leaves("ash")),
        );
        registry.register(
            EntityTypeDef::new("broodling")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20),
        );
        // A 3x3 that bursts when something kills it, and leaves nothing when it
        // goes any other way.
        registry.register(
            EntityTypeDef::new("hive")
                .with_location(utils::GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_health(200)
                .with_dying(
                    2,
                    [Bequest::new(
                        "broodling",
                        2,
                        LeftBy::Named(vec![DeathKind::Killed]),
                    )],
                ),
        );
        // What rides in it: a body-leaving unit carrying the cargo size and
        // the legs that boarding asks for.
        registry.register(
            utils::walker("rider", utils::GROUND)
                .with_health(30)
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
                .with_dying(
                    2,
                    [Bequest::new(
                        "corpse",
                        1,
                        LeftBy::Named(vec![DeathKind::CarriedDown]),
                    )],
                ),
        );
        // A carrier that takes its passengers down with it.
        registry.register(
            utils::walker("wagon", utils::GROUND)
                .with_health(60)
                .with_dying(2, [])
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::from_num(3))
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::from_num(2))
                .with_transporter(
                    Kinds::Any,
                    Affiliation::Own,
                    PassengerFate::Destroy,
                    PassengerConduct::Shelter,
                ),
        );
        // A body that lies briefly and rots into nothing at all, and what
        // leaves one.
        registry.register(
            EntityTypeDef::new("cinder")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(30)),
        );
        registry.register(
            EntityTypeDef::new("imp")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(30)
                .with_dying(2, utils::leaves("cinder")),
        );
        // A 3x3 that leaves a body wherever it falls.
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(utils::GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_health(200)
                .with_dying(2, utils::leaves("corpse")),
        );
        registry.register(
            EntityTypeDef::new("wraith")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_health(30)
                .with_dying(2, utils::leaves("corpse")),
        );
        registry.register(
            EntityTypeDef::new("sapper")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(30)
                .with_dying(
                    2,
                    [Bequest::new(
                        "corpse",
                        1,
                        LeftBy::Named(vec![DeathKind::Killed]),
                    )],
                ),
        );
        // A body that holds the ground it lies on, and what leaves one.
        registry.register(
            EntityTypeDef::new("rubble")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
        );
        registry.register(
            utils::walker("cart", utils::GROUND)
                .with_health(30)
                .with_dying(2, utils::leaves("rubble")),
        );
        registry.register(
            EntityTypeDef::new("mole")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Underfoot)
                .with_health(20),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Kills `victim` outright with a weapon that leaves `slain`, then runs out its
/// dying phase so whatever it leaves is on the ground.
fn kill(app: &mut App, victim: Entity, killer: SimulationId, slain: Slain) {
    damage::apply(
        app.world_mut(),
        killer,
        victim,
        FixedU64::from_num(1000),
        slain,
    );
    utils::run_ticks(app, 4);
}

/// Every standing entity of `type_name`.
fn standing(app: &App, type_name: &str) -> Vec<Entity> {
    let world = app.world();
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .filter(|&entity| {
            world
                .entity(entity)
                .get::<EntityInfoComponent>()
                .is_some_and(|info| info.type_name() == type_name)
        })
        .collect()
}

/// Every body lying on the map.
fn bodies(app: &App) -> Vec<Entity> {
    app.world()
        .resource::<EntityIndex>()
        .remains_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect()
}
