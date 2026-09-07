//! Sites raised over resource sources: the source's amount passes to the
//! site, the site is worked as the source, and the source comes back when the
//! site falls with anything left in it.

mod utils;

use bevy::prelude::*;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        attached::AttachedComponent,
        build::{OverbuiltComponent, UnderConstructionComponent},
        entity_info::EntityInfoComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        resource::ResourceSourceComponent,
    },
    entity_def,
    order::Order,
    simulation_id::SimulationId,
    spawn,
};

//
// ─── Raising a site over a source ─────────────────────────────────────────────
//

#[test]
fn site_rises_over_mine_and_takes_its_gold() {
    let mut app = utils::orders_app();
    let (_, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    let (mine, _) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(10, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 40;
    utils::grant_gold(&mut app, 30);

    order_shaft_house(&mut app, sylph_id, 10, 10);
    // The order lands on the third tick; one cell of walk brings the sylph
    // into reach on the fifth, when the site is placed and paid for.
    utils::run_ticks(&mut app, utils::APPLY + 4);
    let house = utils::single_owned_of_type(app.world_mut(), "shaft_house", 0);
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(house)
            .unwrap()
            .amount,
        40,
        "the mine's gold is the site's now"
    );
    assert!(app.world().get::<OverbuiltComponent>(house).is_some());
    assert_eq!(utils::gold(app.world()), 10);
    utils::assert_despawned(app.world_mut(), mine);
}

#[test]
fn site_needs_source_under_it() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    utils::grant_gold(&mut app, 30);

    order_shaft_house(&mut app, sylph_id, 10, 10);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "shaft_house"), 0);
    assert_eq!(utils::gold(app.world()), 30);
    assert!(utils::order_queue_is_empty(app.world_mut(), sylph));
}

#[test]
fn cancelled_site_gives_mine_back_with_its_gold() {
    let mut app = utils::orders_app();
    let (_, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    let (mine, _) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(10, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 40;
    utils::grant_gold(&mut app, 30);

    order_shaft_house(&mut app, sylph_id, 10, 10);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    let house = utils::single_owned_of_type(app.world_mut(), "shaft_house", 0);
    let site = entity_def::simulation_id(app.world(), house);

    utils::push_command(&mut app, PlayerCommand::CancelBuild { site });
    // Landed on the third tick, dead two ticks later, and the mine stands
    // where the site stood the tick it is gone.
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "shaft_house"), 0);
    assert_eq!(utils::gold(app.world()), 30, "the price came back");
    let mines = utils::count_of_type(app.world_mut(), "mine");
    assert_eq!(mines, 1);
    let uncovered = app
        .world_mut()
        .query::<(Entity, &ResourceSourceComponent)>()
        .iter(app.world())
        .find(|(_, source)| source.amount == 40)
        .map(|(entity, _)| entity)
        .expect("the mine keeps its gold");
    assert_eq!(utils::cell_of(app.world(), uncovered), CellPos::new(10, 10));
}

#[test]
fn finished_site_is_mined_directly_and_leaves_nothing_when_drained() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    let (mine, _) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(10, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 10;
    utils::grant_gold(&mut app, 30);

    order_shaft_house(&mut app, sylph_id, 10, 10);
    // Placed on the fifth tick, and twenty ticks of its own work finish it.
    utils::run_ticks(&mut app, utils::APPLY + 4 + 20);
    let house = utils::single_owned_of_type(app.world_mut(), "shaft_house", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(house)
            .is_none()
    );
    let house_id = entity_def::simulation_id(app.world(), house);

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: house_id,
            flush: true,
        },
    );
    // Seated on the house's own cell on the third tick; two loads of five
    // drain the ten and bank them where it sits.
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());
    assert_eq!(utils::cell_of(app.world(), sylph), CellPos::new(10, 10));
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::gold(app.world()), 20);

    // Drained dry, the house falls, and no mine comes back from nothing.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "shaft_house"), 0);
    assert_eq!(utils::count_of_type(app.world_mut(), "mine"), 0);
    assert!(app.world().get::<AttachedComponent>(sylph).is_none());
}

#[test]
fn drained_site_over_persistent_source_gives_it_back_empty() {
    // A geyser stays on the map when it runs dry. Drained through the pump
    // house raised over it and then destroyed, the pump house gives the empty
    // geyser back; the mine, which is destroyed when drained, gives nothing.
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    let (geyser, _) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(10, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 10;
    utils::grant_gold(&mut app, 30);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: sylph_id,
            type_name: "pump_house".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4 + 20);
    let house = utils::single_owned_of_type(app.world_mut(), "pump_house", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(house)
            .is_none()
    );
    let house_id = entity_def::simulation_id(app.world(), house);

    // Two loads of five drain it; the house falls, and the geyser stands
    // where it stood, with nothing left in it.
    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: house_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4 + 4);
    assert_eq!(utils::gold(app.world()), 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "pump_house"), 0);
    let geysers = app
        .world_mut()
        .query::<(Entity, &EntityInfoComponent, &ResourceSourceComponent)>()
        .iter(app.world())
        .filter(|(_, info, _)| info.type_name() == "geyser")
        .map(|(entity, _, source)| (entity, source.amount))
        .collect::<Vec<_>>();
    let [(uncovered, amount)] = geysers.as_slice() else {
        panic!("one geyser stands again");
    };
    assert_eq!(*amount, 0);
    assert_eq!(
        utils::cell_of(app.world(), *uncovered),
        CellPos::new(10, 10)
    );
    assert!(app.world().get::<AttachedComponent>(sylph).is_none());
}

//
// ─── Working a site as the source it covers ───────────────────────────────────
//

#[test]
fn owned_source_refuses_rival_and_unfinished_site_refuses_its_owner() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    let (rival, _) = utils::create_owned(&mut app, "sylph", 12, 12, 1);
    let (mine, _) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(10, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 40;
    utils::grant_gold(&mut app, 30);

    order_shaft_house(&mut app, sylph_id, 10, 10);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    let house = utils::single_owned_of_type(app.world_mut(), "shaft_house", 0);
    let house_id = entity_def::simulation_id(app.world(), house);

    // Still going up: its own owner's carrier is refused. The order is pushed
    // straight onto the queue, so the refusal is the harvest's own and not a
    // smart send reading the site some other way.
    app.world_mut()
        .get_mut::<OrderQueueComponent>(sylph)
        .unwrap()
        .push(
            Order::Harvest { target: house_id },
            Some(CancelPolicy::Force),
        );
    utils::run_ticks(&mut app, 2);
    assert!(utils::order_queue_is_empty(app.world_mut(), sylph));
    assert!(app.world().get::<AttachedComponent>(sylph).is_none());

    // Finished, it is player 0's source: player 1's carrier has no business
    // there, and its order ends on its first tick.
    utils::run_ticks(&mut app, 18);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(house)
            .is_none()
    );
    app.world_mut()
        .get_mut::<OrderQueueComponent>(rival)
        .unwrap()
        .push(
            Order::Harvest { target: house_id },
            Some(CancelPolicy::Force),
        );
    utils::run_ticks(&mut app, 2);
    assert!(app.world().get::<AttachedComponent>(rival).is_none());
    assert!(utils::order_queue_is_empty(app.world_mut(), rival));
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn unfinished_site_is_never_taken_up_as_replacement_source() {
    let mut app = utils::orders_app();
    let (carrier, carrier_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    let (_, builder_id) = utils::create_owned(&mut app, "sylph", 12, 12, 0);
    // The seam the carrier works, and a second one for a shaft house to rise
    // over three cells away — within the twelve cells a carrier searches.
    let (geyser, geyser_id) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(9, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 3;
    let (mine, _) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(12, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 40;
    utils::grant_gold(&mut app, 20);

    // The site is up first, holding the mine's forty gold and twenty ticks
    // away from finishing.
    order_shaft_house(&mut app, builder_id, 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let house = utils::single_owned_of_type(app.world_mut(), "shaft_house", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(house)
            .is_some(),
        "the site is still going up"
    );

    // The carrier draws its geyser dry and looks around: the unfinished site
    // two cells away is no source to take up.
    utils::select(&mut app, carrier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: geyser_id,
            flush: true,
        },
    );
    // Three ticks for the command, two to draw the geyser dry — the tick it
    // sits down is the first of them — and one more to look around and find
    // nothing it may work.
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(house)
            .is_some(),
        "the site is still going up"
    );
    assert!(app.world().get::<AttachedComponent>(carrier).is_none());
    assert!(utils::order_queue_is_empty(app.world_mut(), carrier));
    // The three the geyser held, and nothing out of the site.
    assert_eq!(utils::gold(app.world_mut()), 3);
}

#[test]
fn uncovered_source_comes_back_where_it_was_covered() {
    let mut app = utils::orders_app();
    let (_, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);
    // A geyser holds its ground when emptied, so it is the one that can be
    // given back after its cover stopped drawing anything.
    let (geyser, _) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(10, 10), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 5;
    utils::grant_gold(&mut app, 20);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: sylph_id,
            type_name: "pump_house".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    // Three ticks for the command, two to walk the two cells, and twenty to
    // raise it — the sylph founds the site and the site finishes on its own.
    utils::run_ticks(&mut app, utils::APPLY + 2 + 20);
    let house = utils::single_owned_of_type(app.world_mut(), "pump_house", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(house)
            .is_none()
    );

    // The builder is sent well clear: the walking form spreads over every cell
    // around the house, so anything standing beside it would refuse the change.
    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(4, 4),
            flush: true,
        },
    );
    // Three ticks for the command, then six cells at half a cell a tick.
    utils::run_ticks(&mut app, utils::APPLY + 12);

    // Uprooted, the pump house is a 3x3 walker centred on the cell it stood on,
    // so its own anchor is (9, 9) — one cell short of the geyser's on both axes.
    app.world_mut()
        .entity_mut(house)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: "walking_pump".into(),
            },
            Some(CancelPolicy::Force),
        );
    // One tick to take the order up, four for the change itself.
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::cell_of(app.world(), house),
        CellPos::new(9, 9),
        "the walker stands a cell out from the ground the house held"
    );

    // Killed, it gives the geyser back on the cell it was covered on, not under
    // the walker's own middle. The walking form draws nothing, so what comes
    // back is empty — and it comes back at all only because a geyser holds its
    // ground when it runs dry.
    spawn::destroy_entity(app.world_mut(), house);
    // Two ticks of dying, then the body is cleared on the next.
    utils::run_ticks(&mut app, 3);
    let geysers = app
        .world_mut()
        .query::<(Entity, &EntityInfoComponent, &ResourceSourceComponent)>()
        .iter(app.world())
        .filter(|(_, info, _)| info.type_name() == "geyser")
        .map(|(entity, _, source)| (entity, source.amount))
        .collect::<Vec<_>>();
    let [(uncovered, amount)] = geysers.as_slice() else {
        panic!("one geyser stands again");
    };
    assert_eq!(*amount, 0);
    assert_eq!(
        utils::cell_of(app.world(), *uncovered),
        CellPos::new(10, 10)
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn order_shaft_house(app: &mut App, builder: SimulationId, x: u32, y: u32) {
    utils::push_command(
        app,
        PlayerCommand::BuildEntity {
            builder,
            type_name: "shaft_house".into(),
            position: utils::pos(x, y),
            flush: true,
        },
    );
}
