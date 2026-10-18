//! Relocation casts: bodies moved at once to new cells, by journey and landing.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    affiliation::Affiliation,
    cost::Cost,
    detection::Detection,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{FieldAction, FieldDecay, FieldDef, FieldLayer, FieldPlacement, FieldVision},
    kinds::Kinds,
    landing::Landing,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry,
    },
    pool::Pool,
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    resource::DepletionPolicy,
    skills::{
        AimParty, Away, CasterParty, Casting, Count, EntityCastEffect, FromAimTo, FromCasterTo,
        GatherSight, Gathering, Journey, PaymentPoint, Reach, Shortfall, SkillCaster, SkillDef,
        Whom,
    },
    transport::{PassengerConduct, PassengerFate},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, facing::Facing};
use ferrets_simulation::{
    command::{PlayerCommand, SkillTarget},
    components::{hidden::Inside, location::LocationComponent, rise::RiseAttendance},
    entity_def,
    events::{EventRecord, SimulationEvent},
    game_loop::{
        cast::{self, AimRefusal},
        fields,
    },
    map::Map,
    movement_model::MovementModel,
    session::{
        GameSession, ai_detection::AiDetection, ai_vision::AiVision, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    spawn::{self, Boarding},
    visibility::{CellVisibility, VisibilityGrid},
};

//
// ─── Blink ───────────────────────────────────────────────────────────────────
//

#[test]
fn blink_moves_caster_to_aimed_cell_keeping_its_facing() {
    let mut app = app();
    utils::record_announcements(&mut app);
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);
    // Turned away from the way it spawns facing, so a facing reset would show.
    app.world_mut()
        .get_mut::<LocationComponent>(mage)
        .expect("a mage stands on the map")
        .facing = Facing::NORTH;

    utils::cast_at(&mut app, mage_id, "blink", 10, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(10, 5));
    assert_eq!(utils::facing_of(app.world(), mage), Facing::NORTH);
    assert_eq!(utils::energy_as_u32(&app, mage), 150, "200 - 50");
    assert_eq!(
        relocations(&app),
        vec![SimulationEvent::EntityRelocated {
            entity: mage_id,
            from: utils::pos(3, 5),
            to: utils::pos(10, 5),
        }],
        "one move, announced once"
    );
}

#[test]
fn blink_onto_held_cell_lands_nearest_it_on_side_caster_came_from() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);
    utils::hold_cell(&mut app, CellPos::new(10, 5));

    utils::cast_at(&mut app, mage_id, "blink", 10, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The first ring around the aim, the cell nearest where it stood first.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(9, 5));
}

#[test]
fn blink_with_nowhere_to_land_is_refused_unpaid() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);

    utils::cast_at(&mut app, mage_id, "blink", 10, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(3, 5));
    assert_eq!(utils::energy_as_u32(&app, mage), 200, "nothing was paid");
}

#[test]
fn sighted_blink_refuses_aim_its_side_does_not_see() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);
    utils::run_ticks(&mut app, 1);

    let skill = utils::skill_id(&app, "blink_sighted");
    assert_eq!(
        cast::aim(
            app.world(),
            0,
            mage,
            skill,
            Some(SkillTarget::Position(utils::pos(28, 28))),
        ),
        Err(AimRefusal::LandingUnseen)
    );
    utils::cast_at(&mut app, mage_id, "blink_sighted", 28, 28);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(3, 5));
    assert_eq!(utils::energy_as_u32(&app, mage), 200, "nothing was paid");
}

#[test]
fn sighted_blink_lands_on_cell_its_side_sees() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);

    utils::cast_at(&mut app, mage_id, "blink_sighted", 10, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(10, 5));
    assert_eq!(utils::energy_as_u32(&app, mage), 150, "200 - 50");
}

#[test]
fn sighted_blink_onto_held_cell_skips_fogged_cells_beside_it() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);
    utils::run_ticks(&mut app, 1);
    for (x, y) in [(13, 5), (12, 4), (12, 5), (12, 6)] {
        utils::hold_cell(&mut app, CellPos::new(x, y));
    }
    let seen = |app: &App, x, y| app.world().resource::<VisibilityGrid>().get(0, x, y);
    assert_eq!(seen(&app, 13, 5), CellVisibility::Visible);
    assert_eq!(seen(&app, 13, 4), CellVisibility::Unexplored);

    utils::cast_at(&mut app, mage_id, "blink_sighted", 13, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Of the first ring around the aim, the mage's side is held and the rest
    // lies in fog; of the second, (11, 5) is nearest where it stood.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(11, 5));
}

#[test]
fn sighted_blink_of_omniscient_seat_lands_in_fog() {
    let mut app = seated_app(
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::occupied(
                1,
                PlayerType::Ai {
                    vision: AiVision::Omniscient,
                    detection: AiDetection::Detectors,
                },
                None,
                None,
            ),
        ],
        1,
    );
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 1);

    // (28, 28) lies far beyond the mage's ten cells of sight; the seat sees
    // the whole map.
    utils::cast_at(&mut app, mage_id, "blink_sighted", 28, 28);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(28, 28));
}

#[test]
fn explored_blink_lands_on_cell_seen_before_but_refuses_one_never_seen() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);
    utils::run_ticks(&mut app, 1);
    let skill = utils::skill_id(&app, "blink_explored");
    assert_eq!(
        cast::aim(
            app.world(),
            0,
            mage,
            skill,
            Some(SkillTarget::Position(utils::pos(28, 28))),
        ),
        Err(AimRefusal::LandingUnseen),
        "never seen"
    );

    // Seen for a tick, then out of sight again: explored, not visible.
    app.world_mut()
        .resource_mut::<VisibilityGrid>()
        .reveal(0, 28, 28);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        app.world().resource::<VisibilityGrid>().get(0, 28, 28),
        CellVisibility::Explored
    );

    utils::cast_at(&mut app, mage_id, "blink_explored", 28, 28);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(28, 28));
}

#[test]
fn blink_landing_anywhere_lands_in_fog() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);

    utils::cast_at(&mut app, mage_id, "blink", 28, 28);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(28, 28));
}

//
// ─── Recall ──────────────────────────────────────────────────────────────────
//

#[test]
fn recalled_unit_lands_only_where_its_placement_rules_admit_it() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (sapper, _) = utils::create_owned(&mut app, "sapper", 20, 5, 0);
    // Creep on the cell it would take otherwise, the one nearest where it
    // stood.
    let creep = app
        .world()
        .resource::<ContentRegistry>()
        .field("creep")
        .expect("creep is registered");
    fields::apply_action(
        app.world_mut(),
        1,
        creep,
        CellPos::new(6, 5),
        0,
        FieldAction::Cover,
    );

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), sapper), CellPos::new(6, 4));
}

#[test]
fn recall_moves_those_that_fit_and_leaves_rest_standing() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (first, _) = utils::create_owned(&mut app, "footman", 20, 5, 0);
    let (second, _) = utils::create_owned(&mut app, "footman", 20, 6, 0);
    let (third, _) = utils::create_owned(&mut app, "footman", 20, 7, 0);
    // Room beside the mage for two.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::free_cell(&mut app, CellPos::new(6, 5));
    utils::free_cell(&mut app, CellPos::new(6, 6));

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Nearest the aim first, each on the side of the mage it came from.
    assert_eq!(utils::cell_of(app.world(), first), CellPos::new(6, 5));
    assert_eq!(utils::cell_of(app.world(), second), CellPos::new(6, 6));
    assert_eq!(
        utils::cell_of(app.world(), third),
        CellPos::new(20, 7),
        "the one that fit nowhere stays"
    );
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(5, 5));
    assert_eq!(utils::energy_as_u32(&app, mage), 150, "200 - 50, paid");
}

#[test]
fn refusing_recall_with_one_that_fits_nowhere_moves_nobody_unpaid() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (first, _) = utils::create_owned(&mut app, "footman", 20, 5, 0);
    let (second, _) = utils::create_owned(&mut app, "footman", 20, 6, 0);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::free_cell(&mut app, CellPos::new(6, 5));

    utils::cast_at(&mut app, mage_id, "recall_refuse", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), first), CellPos::new(20, 5));
    assert_eq!(utils::cell_of(app.world(), second), CellPos::new(20, 6));
    assert_eq!(utils::energy_as_u32(&app, mage), 200, "nothing was paid");
}

#[test]
fn recall_gathering_nobody_is_refused_unpaid() {
    let mut app = app();
    utils::record_announcements(&mut app);
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::energy_as_u32(&app, mage), 200, "nothing was paid");
    assert!(relocations(&app).is_empty());
}

#[test]
fn anyones_gathering_takes_resource_source_with_harvester_inside() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (mine, _) = utils::create_entity(app.world_mut(), "mine", utils::pos(20, 5), None)
        .expect("the fixture defines a mine");
    let (digger, _) = utils::create_owned(&mut app, "footman", 22, 5, 0);
    let mine_id = entity_def::simulation_id(app.world(), mine);
    spawn::hide_entity(app.world_mut(), digger, Inside::Job(mine_id));

    utils::cast_at(&mut app, mage_id, "recall_blind", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the mage on the side it came from, the digger still inside it.
    assert_eq!(utils::cell_of(app.world(), mine), CellPos::new(6, 5));
    assert_eq!(entity_def::inside(app.world(), digger), Some(mine));
}

#[test]
fn gathering_takes_body_standing_part_way_into_its_radius() {
    let mut app = app();
    let projection = app.world().resource::<Map>().projection();
    utils::install_map(&mut app, projection, MovementModel::Continuous);
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (footman, footman_id) = utils::create_owned(&mut app, "footman", 15, 5, 0);
    // Floored into (17, 5), three cells off the aim at (20, 5); rounded to
    // (18, 5), two off, inside the recall's two-cell radius.
    let corner = utils::part_way("17.6", "5");
    utils::select(&mut app, footman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: corner,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::position_of(app.world(), footman), corner);

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the mage at (5, 5), on the side it came from.
    assert_eq!(utils::cell_of(app.world(), footman), CellPos::new(6, 5));
}

#[test]
fn gathering_at_most_two_takes_nearest_then_lowest_id() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (nearest, _) = utils::create_owned(&mut app, "footman", 20, 5, 0);
    let (lower_id, _) = utils::create_owned(&mut app, "footman", 20, 6, 0);
    let (higher_id, _) = utils::create_owned(&mut app, "footman", 20, 4, 0);

    utils::cast_at(&mut app, mage_id, "recall_two", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the mage, each on the side of the mage it came from.
    assert_eq!(utils::cell_of(app.world(), nearest), CellPos::new(6, 5));
    assert_eq!(utils::cell_of(app.world(), lower_id), CellPos::new(6, 6));
    assert_eq!(
        utils::cell_of(app.world(), higher_id),
        CellPos::new(20, 4),
        "a step away like the second, but with the higher id"
    );
}

#[test]
fn gathering_never_takes_rising_entity() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (rising, _) = utils::create_rising(&mut app, "hall", 20, 5, 0, RiseAttendance::Halted);

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        utils::cell_of(app.world(), rising),
        CellPos::new(20, 5),
        "a rising entity stays where it stands"
    );
}

#[test]
fn gathering_takes_finished_hall_its_filter_admits() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (hall, _) = utils::create_owned(&mut app, "hall", 20, 5, 0);

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The 2×2 hall stands beside the mage on the side it came from, at
    // (6..8, 5..7).
    assert_eq!(utils::cell_of(app.world(), hall), CellPos::new(6, 5));
}

#[test]
fn gathered_halls_land_clear_of_what_stands_underfoot() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (first, _) = utils::create_owned(&mut app, "hall", 20, 5, 0);
    let (second, _) = utils::create_owned(&mut app, "hall", 20, 7, 0);
    // The mole claims no cell but holds its ground: no hall is raised over
    // it, where the first would land otherwise, at (6..8, 5..7).
    utils::create_owned(&mut app, "mole", 6, 5, 0);

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The first takes (6..8, 6..8), clear of the mole; the second, gathered
    // in the same cast, the next ground beside the mage that takes it,
    // (4..6, 6..8).
    assert_eq!(utils::cell_of(app.world(), first), CellPos::new(6, 6));
    assert_eq!(utils::cell_of(app.world(), second), CellPos::new(4, 6));
}

#[test]
fn gathering_never_takes_unit_changing_form() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (changing, changing_id) = utils::create_owned(&mut app, "footman", 20, 5, 0);
    let (standing, _) = utils::create_owned(&mut app, "footman", 20, 6, 0);
    utils::command_morph(&mut app, changing_id, "veteran");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The one changing stays; the other lands beside the mage nearest where
    // it stood.
    assert_eq!(utils::cell_of(app.world(), changing), CellPos::new(20, 5));
    assert_eq!(utils::cell_of(app.world(), standing), CellPos::new(6, 6));
}

#[test]
fn seen_gathering_takes_only_what_its_side_makes_out() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (seen, _) = utils::create_owned(&mut app, "footman", 15, 5, 1);
    let (fogged, _) = utils::create_owned(&mut app, "footman", 16, 5, 1);
    utils::run_ticks(&mut app, 1);
    let grid = app.world().resource::<VisibilityGrid>();
    assert_eq!(grid.get(0, 15, 5), CellVisibility::Visible);
    assert_eq!(grid.get(0, 16, 5), CellVisibility::Unexplored);

    utils::cast_at(&mut app, mage_id, "recall_seen", 15, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), seen), CellPos::new(6, 5));
    assert_eq!(utils::cell_of(app.world(), fogged), CellPos::new(16, 5));
}

#[test]
fn blind_gathering_takes_fogged_units_as_well_as_seen() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (seen, _) = utils::create_owned(&mut app, "footman", 15, 5, 1);
    let (fogged, _) = utils::create_owned(&mut app, "footman", 16, 5, 1);

    utils::cast_at(&mut app, mage_id, "recall_blind", 15, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The one on the aim first, on the mage's side toward it; the fogged one
    // on the next cell of the ring, first by row.
    assert_eq!(utils::cell_of(app.world(), seen), CellPos::new(6, 5));
    assert_eq!(utils::cell_of(app.world(), fogged), CellPos::new(6, 4));
}

#[test]
fn relocated_transport_carries_its_passengers_aboard() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 20, 5, 0);
    let (rider, rider_id) = utils::create_owned(&mut app, "footman", 21, 5, 0);
    assert_eq!(
        spawn::board(app.world_mut(), rider, wagon),
        Boarding::Boarded
    );

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        utils::cell_of(app.world(), wagon),
        CellPos::new(6, 5),
        "the wagon is recalled beside the mage"
    );
    assert_eq!(
        utils::passengers_of(app.world(), wagon),
        vec![rider_id],
        "its rider rides along aboard, not set down on its own"
    );
    assert!(entity_def::hidden(app.world(), rider));
}

#[test]
fn relocated_transport_unloads_passengers_beside_its_new_cell() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 20, 5, 0);
    let (rider, _) = utils::create_owned(&mut app, "footman", 21, 5, 0);
    assert_eq!(
        spawn::board(app.world_mut(), rider, wagon),
        Boarding::Boarded
    );
    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::cell_of(app.world(), wagon), CellPos::new(6, 5));

    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Of the ring around the wagon's new cell, the four sides lie nearest its
    // middle, (6, 4) first by row.
    assert!(!entity_def::hidden(app.world(), rider));
    assert_eq!(utils::cell_of(app.world(), rider), CellPos::new(6, 4));
}

#[test]
fn recalled_unit_drops_its_walk() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (footman, _) = utils::create_owned(&mut app, "footman", 20, 5, 0);
    // Walking east, a cell and a half along when the recall lands.
    utils::order_move(&mut app, footman, 26, 5);

    utils::cast_at(&mut app, mage_id, "recall_leave", 20, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        utils::cell_of(app.world(), footman),
        CellPos::new(6, 5),
        "recalled beside the mage"
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), footman),
        "and its walk is dropped"
    );
}

#[test]
fn caster_moved_by_its_own_cast_carries_on_with_what_it_queued_after() {
    let mut app = app();
    utils::record_announcements(&mut app);
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 3, 5, 0);

    utils::cast_at(&mut app, mage_id, "slow_blink", 10, 5);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // Queued behind the cast under way: blink, then walk up to (10, 1).
    utils::order_move(&mut app, mage, 10, 1);
    utils::run_ticks(&mut app, 10);

    assert_eq!(
        relocations(&app),
        vec![SimulationEvent::EntityRelocated {
            entity: mage_id,
            from: utils::pos(3, 5),
            to: utils::pos(10, 5),
        }],
        "the blink landed"
    );
    // Four cells up from the landing at half a cell a tick takes eight ticks;
    // from where it cast, the walk would still be under way.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(10, 1));
    assert!(
        utils::order_queue_is_empty(app.world_mut(), mage),
        "and the walk is done"
    );
}

//
// ─── Pull, swap and portal ───────────────────────────────────────────────────
//

#[test]
fn pull_brings_target_beside_caster() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (footman, footman_id) = utils::create_owned(&mut app, "footman", 20, 5, 0);

    utils::cast_on(&mut app, mage_id, "pull", footman_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the mage on the side it came from.
    assert_eq!(utils::cell_of(app.world(), footman), CellPos::new(6, 5));
}

#[test]
fn pull_aimed_at_its_own_caster_moves_nothing() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);

    utils::cast_on(&mut app, mage_id, "pull", mage_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(5, 5));
    assert_eq!(utils::energy_as_u32(&app, mage), 200, "nothing was paid");
}

#[test]
fn body_mid_crossing_lifts_cell_its_crossing_claimed() {
    let mut app = app();
    let (snail, snail_id) = utils::create_owned(&mut app, "snail", 5, 5, 0);
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 9, 5, 0);
    // Every cell held but a corridor west of the snail and the mage's own.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    for x in 1..=4 {
        utils::free_cell(&mut app, CellPos::new(x, 5));
    }
    utils::free_cell(&mut app, CellPos::new(9, 5));
    utils::cast_on(&mut app, mage_id, "swap", snail_id);
    utils::run_ticks(&mut app, utils::APPLY - 1);
    // A quarter cell a tick: the swap lands a quarter of the way to (4, 5),
    // the cell its crossing claimed, nearer the cell it left.
    utils::order_move(&mut app, snail, 1, 5);
    utils::run_ticks(&mut app, 1);

    // Nearest (5, 5) on the claimed cell, lifted with the snail.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(4, 5));
    assert_eq!(utils::cell_of(app.world(), snail), CellPos::new(9, 5));
}

#[test]
fn swap_trades_caster_and_target_places() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (footman, footman_id) = utils::create_owned(&mut app, "footman", 12, 9, 0);

    utils::cast_on(&mut app, mage_id, "swap", footman_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(12, 9));
    assert_eq!(utils::cell_of(app.world(), footman), CellPos::new(5, 5));
}

#[test]
fn swap_with_hall_anchors_each_where_other_stood() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (hall, hall_id) = utils::create_owned(&mut app, "hall", 20, 20, 0);

    utils::cast_on(&mut app, mage_id, "swap", hall_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Each footprint is lifted for the other, so the mage takes the hall's
    // anchor and the 2×2 hall the mage's, over (5..7, 5..7).
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(20, 20));
    assert_eq!(utils::cell_of(app.world(), hall), CellPos::new(5, 5));
}

#[test]
fn swap_with_hall_that_fits_nowhere_is_refused_unpaid() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (hall, hall_id) = utils::create_owned(&mut app, "hall", 20, 20, 0);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);

    utils::cast_on(&mut app, mage_id, "swap", hall_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(5, 5));
    assert_eq!(utils::cell_of(app.world(), hall), CellPos::new(20, 20));
    assert_eq!(utils::energy_as_u32(&app, mage), 200, "nothing was paid");
}

#[test]
fn portal_takes_caster_and_those_around_it_to_aimed_hall() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (footman, _) = utils::create_owned(&mut app, "footman", 6, 5, 0);
    let (_, hall_id) = utils::create_owned(&mut app, "hall", 20, 20, 0);

    utils::cast_on(&mut app, mage_id, "portal", hall_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The hall holds (20..22, 20..22): the mage first, on the corner nearest
    // where it stood, then the footman on the next cell nearest where it
    // stood — (20, 19), whose middle lies nearer (6, 5) than (19, 20)'s.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(19, 19));
    assert_eq!(utils::cell_of(app.world(), footman), CellPos::new(20, 19));
}

//
// ─── Home ────────────────────────────────────────────────────────────────────
//

#[test]
fn homeward_goes_to_nearest_finished_hall() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    // A hall still rising, nearer than either risen hall, is no home yet.
    utils::create_rising(&mut app, "hall", 8, 5, 0, RiseAttendance::Halted);
    utils::create_owned(&mut app, "hall", 14, 5, 0);
    utils::create_owned(&mut app, "hall", 25, 25, 0);

    utils::cast_self(&mut app, mage_id, "homeward");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the hall at (14..16, 5..7), on the side it came from.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(13, 5));
}

#[test]
fn homeward_party_lands_caster_first_then_each_nearest_where_it_stood() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (footman, _) = utils::create_owned(&mut app, "footman", 6, 6, 0);
    utils::create_owned(&mut app, "hall", 14, 5, 0);
    utils::create_owned(&mut app, "hall", 25, 25, 0);

    utils::cast_self(&mut app, mage_id, "homeward_party");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the hall at (14..16, 5..7) nearer the mage: the mage first, on
    // the side it came from, then the footman on the next cell nearest where
    // it stood.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(13, 5));
    assert_eq!(utils::cell_of(app.world(), footman), CellPos::new(13, 6));
}

#[test]
fn homeward_party_goes_together_to_hall_nearest_its_caster() {
    let mut app = app();
    // Hall (2..4, 5..7) lies five cells from the mage and (16..18, 5..7)
    // eight; the footman stands nearer the second, six cells from it and
    // seven from the first.
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 8, 5, 0);
    let (footman, _) = utils::create_owned(&mut app, "footman", 10, 5, 0);
    utils::create_owned(&mut app, "hall", 2, 5, 0);
    utils::create_owned(&mut app, "hall", 16, 5, 0);

    utils::cast_self(&mut app, mage_id, "homeward_party");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Both beside the first hall: the mage on the side it came from, the
    // footman on the next cell nearest where it stood.
    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(4, 5));
    assert_eq!(utils::cell_of(app.world(), footman), CellPos::new(4, 6));
}

#[test]
fn sent_home_unit_goes_to_hall_nearest_aim() {
    let mut app = app();
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (footman, footman_id) = utils::create_owned(&mut app, "footman", 22, 5, 0);
    // Hall (8..10, 5..7) lies nearest the mage, (25..27, 5..7) nearest the
    // footman.
    utils::create_owned(&mut app, "hall", 8, 5, 0);
    utils::create_owned(&mut app, "hall", 25, 5, 0);

    utils::cast_on(&mut app, mage_id, "send_home", footman_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the hall at (25..27, 5..7), on the side it came from.
    assert_eq!(utils::cell_of(app.world(), footman), CellPos::new(24, 5));
}

#[test]
fn homeward_with_no_hall_is_refused_unpaid() {
    let mut app = app();
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    // Another player's hall is no home of the mage's.
    utils::create_owned(&mut app, "hall", 14, 5, 1);

    utils::cast_self(&mut app, mage_id, "homeward");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), mage), CellPos::new(5, 5));
    assert_eq!(utils::energy_as_u32(&app, mage), 200, "nothing was paid");
}

//
// ─── Seated workers ──────────────────────────────────────────────────────────
//

#[test]
fn relocated_job_carries_its_seated_workers_onto_its_new_berths() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grubs = utils::owned_of_type(app.world_mut(), "grub", 0);
    assert_eq!(grubs.len(), 1, "the hatch has bred into its berths");
    let before: Vec<_> = grubs
        .iter()
        .map(|&grub| (grub, utils::position_of(app.world(), grub)))
        .collect();

    // Ten cells right and ten down, at once.
    spawn::relocate(app.world_mut(), &[(hatch, CellPos::new(20, 20))]);

    let announced = app.world().resource::<EventRecord>().events().to_vec();
    for (grub, from) in before {
        let to = utils::position_of(app.world(), grub);
        assert_eq!(
            to,
            from + utils::pos(10, 10),
            "each sits on its berth as the hatch now stands, the same tick"
        );
        assert!(
            announced.contains(&SimulationEvent::EntityRelocated {
                entity: entity_def::simulation_id(app.world(), grub),
                from,
                to,
            }),
            "and its move is announced"
        );
    }
}

#[test]
fn relocated_job_carries_dying_seated_worker_along() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = utils::single_owned_of_type(app.world_mut(), "grub", 0);
    // Dying for two ticks, the grub keeps its berth meanwhile.
    spawn::destroy_entity(app.world_mut(), grub);
    assert!(entity_def::attached(app.world(), grub));
    let before = utils::position_of(app.world(), grub);

    spawn::relocate(app.world_mut(), &[(hatch, CellPos::new(20, 20))]);

    // Ten cells right and ten down, with the hatch.
    assert_eq!(
        utils::position_of(app.world(), grub),
        before + utils::pos(10, 10)
    );
    assert!(entity_def::attached(app.world(), grub));
}

#[test]
fn gathering_takes_hatch_carrying_its_seated_grubs_but_never_grubs_alone() {
    let mut app = utils::brood_app(MovementModel::Cell);
    register_recaller(&mut app);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let (_, recaller_id) = utils::create_owned(&mut app, "recaller", 25, 25, 0);
    // The first grub is bred on the tenth tick; the recall lands on the
    // fourteenth, before the second is bred.
    utils::run_ticks(&mut app, 10);
    let grub = utils::single_owned_of_type(app.world_mut(), "grub", 0);
    assert!(entity_def::attached(app.world(), grub));
    let before = utils::position_of(app.world(), grub);

    utils::cast_at(&mut app, recaller_id, "recall", 11, 11);
    utils::run_ticks(&mut app, utils::APPLY);

    // The 3×3 hatch lands on the recaller's ring at (26..29, 24..27), sixteen
    // cells right and fourteen down, its grub put on its berth there in the
    // landing tick rather than moved on its own.
    assert_eq!(utils::cell_of(app.world(), hatch), CellPos::new(26, 24));
    assert!(entity_def::attached(app.world(), grub));
    assert_eq!(
        utils::position_of(app.world(), grub),
        before + utils::pos(16, 14)
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Ticks a `footman` takes to change into a `veteran`.
const CHANGE_TIME: u32 = 20;

/// App with two players: a `mage` seeing ten cells around, with 200 energy
/// and every relocation below, each costing 50; walking `footman`s, each
/// changing into a `veteran`; a `sapper` that never stands on lasting
/// `creep`; a 2×2 `hall`, buildable; a `mole` that holds its ground without
/// claiming its cell; a `snail` crawling a quarter cell a tick; a `wagon`
/// that carries footmen; and an ownerless gold `mine`.
fn app() -> App {
    seated_app(
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::occupied(1, PlayerType::Human, None, None),
        ],
        0,
    )
}

/// The app on `slots`, commanded as `local`.
fn seated_app(slots: Vec<PlayerSlot>, local: PlayerId) -> App {
    let mut app = utils::make_app_as(slots, local);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_tag("hall");
        registry.register_tag("infantry");
        registry.register_resource("gold");
        registry.register(
            EntityTypeDef::new("mine")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Persist),
        );
        registry.register(
            EntityTypeDef::new("hall")
                .with_location(utils::GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_pool(Pool::health(100))
                .with_rise(10, utils::rise_initial_pools(&[PoolId::HEALTH]))
                .with_tags(["hall"]),
        );
        for name in ["footman", "veteran"] {
            registry.register(
                utils::walker(name, utils::GROUND)
                    .with_pool(Pool::health(20))
                    .with_dying(1, [])
                    .with_tags(["infantry"])
                    .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
                    .with_morphs(match name {
                        "footman" => vec![MorphTransition::new(
                            "veteran",
                            MorphCourse::direct(MorphInterrupted::Reverts),
                            Quantity::Constant(CHANGE_TIME),
                            MorphPlacement::Reserve,
                            MorphCancel::Refundable,
                            MorphReason::Change,
                            Vec::new(),
                            Vec::new(),
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                        )],
                        _ => Vec::new(),
                    }),
            );
        }
        let creep = registry.register_field(
            "creep",
            FieldDef::new(
                FieldLayer::Passable(utils::GROUND.into()),
                FieldDecay::Never,
                FieldVision::Dark,
                Detection::Blind,
            ),
        );
        registry.register(
            utils::walker("sapper", utils::GROUND)
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_field_placement([FieldPlacement::Forbids { field: creep }]),
        );
        registry.register(
            EntityTypeDef::new("snail")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.25),
                    FixedU64::from_num(0.25),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20)),
        );
        registry.register(
            EntityTypeDef::new("mole")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Underfoot)
                .with_pool(Pool::health(20)),
        );
        registry.register(
            utils::walker("wagon", utils::GROUND)
                .with_pool(Pool::health(60))
                .with_dying(1, [])
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ONE)
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ONE)
                .with_transporter(
                    Kinds::tags(["infantry"]),
                    Affiliation::Own,
                    PassengerFate::Destroy,
                    PassengerConduct::Shelter,
                ),
        );
        let cell = || Away::Cell {
            reach: Reach::Wherever,
        };
        let own = || Whom {
            side: Affiliation::Own,
            kinds: Kinds::Any,
        };
        let gathering_of = |side, sight, count, shortfall| Gathering {
            radius: Quantity::Constant(2),
            side,
            kinds: Kinds::Any,
            sight,
            count,
            shortfall,
        };
        let gathering =
            |count, shortfall| gathering_of(Affiliation::Own, GatherSight::Blind, count, shortfall);
        let recall_anyones = |sight| Journey::FromAim {
            party: AimParty::Around {
                at: cell(),
                gathering: gathering_of(Affiliation::Anyone, sight, Count::All, Shortfall::Leave),
            },
            to: FromAimTo::Caster,
        };
        let skills = [
            (
                "blink",
                Journey::FromCaster {
                    party: CasterParty::Caster,
                    to: FromCasterTo::Aim(cell()),
                },
                Landing::Anywhere,
            ),
            (
                "blink_sighted",
                Journey::FromCaster {
                    party: CasterParty::Caster,
                    to: FromCasterTo::Aim(cell()),
                },
                Landing::Sighted,
            ),
            (
                "blink_explored",
                Journey::FromCaster {
                    party: CasterParty::Caster,
                    to: FromCasterTo::Aim(cell()),
                },
                Landing::Explored,
            ),
            (
                "recall_leave",
                Journey::FromAim {
                    party: AimParty::Around {
                        at: cell(),
                        gathering: gathering(Count::All, Shortfall::Leave),
                    },
                    to: FromAimTo::Caster,
                },
                Landing::Anywhere,
            ),
            (
                "recall_refuse",
                Journey::FromAim {
                    party: AimParty::Around {
                        at: cell(),
                        gathering: gathering(Count::All, Shortfall::Refuse),
                    },
                    to: FromAimTo::Caster,
                },
                Landing::Anywhere,
            ),
            (
                "recall_two",
                Journey::FromAim {
                    party: AimParty::Around {
                        at: cell(),
                        gathering: gathering(
                            Count::AtMost(Quantity::Constant(2)),
                            Shortfall::Leave,
                        ),
                    },
                    to: FromAimTo::Caster,
                },
                Landing::Anywhere,
            ),
            (
                "pull",
                Journey::FromAim {
                    party: AimParty::Target {
                        whom: own(),
                        reach: Reach::Wherever,
                    },
                    to: FromAimTo::Caster,
                },
                Landing::Anywhere,
            ),
            (
                "recall_seen",
                recall_anyones(GatherSight::Seen),
                Landing::Anywhere,
            ),
            (
                "recall_blind",
                recall_anyones(GatherSight::Blind),
                Landing::Anywhere,
            ),
            (
                "swap",
                Journey::Swap {
                    whom: own(),
                    reach: Reach::Wherever,
                },
                Landing::Anywhere,
            ),
            (
                "portal",
                Journey::FromCaster {
                    party: CasterParty::CasterAndAround(gathering(Count::All, Shortfall::Leave)),
                    to: FromCasterTo::Aim(Away::Standing {
                        whom: Whom {
                            side: Affiliation::Own,
                            kinds: Kinds::tags(["hall"]),
                        },
                        reach: Reach::Wherever,
                    }),
                },
                Landing::Anywhere,
            ),
            (
                "homeward",
                Journey::FromCaster {
                    party: CasterParty::Caster,
                    to: FromCasterTo::Home(Kinds::tags(["hall"])),
                },
                Landing::Anywhere,
            ),
            (
                "send_home",
                Journey::FromAim {
                    party: AimParty::Target {
                        whom: own(),
                        reach: Reach::Wherever,
                    },
                    to: FromAimTo::Home(Kinds::tags(["hall"])),
                },
                Landing::Anywhere,
            ),
            (
                "homeward_party",
                Journey::FromCaster {
                    party: CasterParty::CasterAndAround(gathering(Count::All, Shortfall::Leave)),
                    to: FromCasterTo::Home(Kinds::tags(["hall"])),
                },
                Landing::Anywhere,
            ),
        ];
        let mut ids: Vec<_> = skills
            .into_iter()
            .map(|(name, journey, landing)| {
                registry.register_skill(
                    name,
                    SkillDef {
                        cooldown: 10,
                        caster: SkillCaster::Entity {
                            costs: vec![Cost::Energy(FixedU64::from_num(50))],
                            casting: Casting::Instant,
                            effect: EntityCastEffect::Relocate {
                                journey: Box::new(journey),
                                landing,
                            },
                        },
                        requires: Vec::new(),
                    },
                )
            })
            .collect();
        ids.push(registry.register_skill(
            "slow_blink",
            SkillDef {
                cooldown: 10,
                caster: SkillCaster::Entity {
                    costs: vec![Cost::Energy(FixedU64::from_num(50))],
                    casting: Casting::Delayed {
                        point: Quantity::Constant(3),
                        period: Quantity::Constant(3),
                        pays: PaymentPoint::AtLanding,
                    },
                    effect: EntityCastEffect::Relocate {
                        journey: Box::new(Journey::FromCaster {
                            party: CasterParty::Caster,
                            to: FromCasterTo::Aim(cell()),
                        }),
                        landing: Landing::Anywhere,
                    },
                },
                requires: Vec::new(),
            },
        ));
        registry.register(
            utils::walker("mage", utils::GROUND)
                .with_pool(Pool::health(40))
                .with_pool(Pool::energy(200))
                .with_sight_range(10)
                .with_skills(ids),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Registers a free instant `recall` gathering everything within two cells
/// of the aim around its caster, and a walking `recaller` casting it.
fn register_recaller(app: &mut App) {
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    let recall = registry.register_skill(
        "recall",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Relocate {
                    journey: Box::new(Journey::FromAim {
                        party: AimParty::Around {
                            at: Away::Cell {
                                reach: Reach::Wherever,
                            },
                            gathering: Gathering {
                                radius: Quantity::Constant(2),
                                side: Affiliation::Own,
                                kinds: Kinds::Any,
                                sight: GatherSight::Blind,
                                count: Count::All,
                                shortfall: Shortfall::Leave,
                            },
                        },
                        to: FromAimTo::Caster,
                    }),
                    landing: Landing::Anywhere,
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::walker("recaller", utils::GROUND)
            .with_pool(Pool::health(40))
            .with_skills([recall]),
    );
    registry.validate();
}

/// Every relocation announced so far.
fn relocations(app: &App) -> Vec<SimulationEvent> {
    app.world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter(|event| matches!(event, SimulationEvent::EntityRelocated { .. }))
        .cloned()
        .collect()
}
