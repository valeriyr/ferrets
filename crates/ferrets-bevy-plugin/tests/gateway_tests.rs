//! Gateways: bodies entering a network or crossing a tunnel to its other end.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::game_view;
use ferrets_content::{
    affiliation::Affiliation,
    berths::BerthGroup,
    brood::OrphanFate,
    build::BuilderAttendance,
    detection::Detection,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{FieldAction, FieldDecay, FieldDef, FieldPlacement, FieldVision},
    gateway::{Doors, Joins},
    kinds::Kinds,
    landing::Landing,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry, RevertCarry, ViaInterrupted,
    },
    network::NetworkDef,
    pool::Pool,
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    skills::{Casting, EntityCastEffect, Reach, SkillCaster, SkillDef, SummonArrival, SummonAt},
    terrains::Terrains,
    transport::{PassengerConduct, PassengerFate},
    tunnel::{OtherEndLost, Passage, TunnelDef},
    work::{Attachment, BerthStance},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    beside::Lean,
    bore_id::BoreId,
    command::PlayerCommand,
    components::{
        hidden::HiddenComponent,
        rally::RallyTarget,
        tunnel::{OtherEnd, TunnelEndComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    events::SimulationEvent,
    game_loop::{
        fields,
        warp::{self, WarpRefusal},
    },
    links::{self, LinkKey, LinkPassengers, Membership},
    map::Map,
    movement_model::MovementModel,
    networks::NetworkScope,
    session::{
        ai_detection::AiDetection, ai_vision::AiVision, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    spawn::{self, Boarding},
    statistics::Statistics,
};

//
// ─── Binding ─────────────────────────────────────────────────────────────────
//

#[test]
fn tunnel_end_trained_by_unbound_end_is_bound_to_it_sharing_its_bore() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);

    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);

    let raised = other_of(&mut app, "tunnel", raiser);
    let raised_id = entity_def::simulation_id(app.world(), raised);
    // The raiser minted bore 1 at birth; the raised end's own, 2, gave way.
    assert_eq!(
        end_of(&app, raiser),
        (OtherEnd::Bound(raised_id), BoreId(1)),
        "the raiser is bound to what it raised, on its own bore"
    );
    assert_eq!(
        end_of(&app, raised),
        (OtherEnd::Bound(raiser_id), BoreId(1)),
        "and the raised end shares the raiser's bore"
    );
}

#[test]
fn tunnel_end_founds_its_other_end_where_ordered_bound_to_it() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);

    utils::build_entity(&mut app, raiser_id, "tunnel", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY + RISE_TIME + 1);

    let raised = other_of(&mut app, "tunnel", raiser);
    let raised_id = entity_def::simulation_id(app.world(), raised);
    assert_eq!(utils::cell_of(app.world(), raised), CellPos::new(20, 20));
    assert!(!entity_def::rising(app.world(), raised));
    // The raiser minted bore 1 at birth; the site's own, 2, gave way.
    assert_eq!(
        end_of(&app, raiser),
        (OtherEnd::Bound(raised_id), BoreId(1))
    );
    assert_eq!(
        end_of(&app, raised),
        (OtherEnd::Bound(raiser_id), BoreId(1))
    );
}

#[test]
fn bound_tunnel_end_refuses_to_train_another() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);
    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);

    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::train_queue_len(app.world(), raiser), 0);
    assert_eq!(utils::owned_of_type(app.world_mut(), "tunnel", 0).len(), 2);
}

#[test]
fn tunnel_end_warped_in_by_unbound_end_is_bound_to_it_sharing_its_bore() {
    let mut app = app();
    let (rift, rift_id) = utils::create_owned(&mut app, "rift", 5, 5, 0);

    utils::warp_in(&mut app, rift_id, "tunnel", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY);

    let warped = utils::single_owned_of_type(app.world_mut(), "tunnel", 0);
    let warped_id = entity_def::simulation_id(app.world(), warped);
    // The rift minted bore 1 at birth; the warped end's own, 2, gave way.
    assert_eq!(end_of(&app, rift), (OtherEnd::Bound(warped_id), BoreId(1)));
    assert_eq!(end_of(&app, warped), (OtherEnd::Bound(rift_id), BoreId(1)));
}

#[test]
fn bound_tunnel_end_refuses_to_warp_in_another() {
    let mut app = app();
    let (rift, rift_id) = utils::create_owned(&mut app, "rift", 5, 5, 0);
    utils::warp_in(&mut app, rift_id, "tunnel", 20, 20);
    // Past the rift's cooldown, so only its binding refuses.
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);

    assert_eq!(
        warp::judge(app.world(), 0, rift, "tunnel", utils::pos(25, 25)).map(|_| ()),
        Err(WarpRefusal::Bound)
    );
}

#[test]
fn tunnel_end_summoned_by_unbound_shaft_is_bound_to_it_sharing_its_bore() {
    let mut app = app();
    let (shaft, shaft_id) = utils::create_owned(&mut app, "shaft", 5, 5, 0);

    utils::cast_at(&mut app, shaft_id, "dig", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let raised = utils::single_owned_of_type(app.world_mut(), "tunnel", 0);
    let raised_id = entity_def::simulation_id(app.world(), raised);
    assert_eq!(utils::cell_of(app.world(), raised), CellPos::new(20, 20));
    // The shaft minted bore 1 at birth; the summoned end's own, 2, gave way.
    assert_eq!(end_of(&app, shaft), (OtherEnd::Bound(raised_id), BoreId(1)));
    assert_eq!(end_of(&app, raised), (OtherEnd::Bound(shaft_id), BoreId(1)));
}

#[test]
fn tunnel_end_raised_from_corpse_by_unbound_shaft_is_bound_to_it() {
    let mut app = app();
    let (shaft, shaft_id) = utils::create_owned(&mut app, "shaft", 5, 5, 0);
    let corpse = corpse_at(&mut app, 8, 5);

    utils::cast_on(&mut app, shaft_id, "exhume", corpse);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let raised = utils::single_owned_of_type(app.world_mut(), "tunnel", 0);
    let raised_id = entity_def::simulation_id(app.world(), raised);
    assert_eq!(end_of(&app, shaft), (OtherEnd::Bound(raised_id), BoreId(1)));
    assert_eq!(end_of(&app, raised), (OtherEnd::Bound(shaft_id), BoreId(1)));
}

#[test]
fn bound_shaft_refuses_to_summon_another() {
    let mut app = app();
    let (_, shaft_id) = utils::create_owned(&mut app, "shaft", 5, 5, 0);
    utils::cast_at(&mut app, shaft_id, "dig", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::cast_at(&mut app, shaft_id, "dig", 25, 25);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    assert_eq!(utils::owned_of_type(app.world_mut(), "tunnel", 0).len(), 1);
}

#[test]
fn bound_tunnel_end_refuses_to_found_another() {
    let mut app = app();
    let ((_, raiser_id), _) = tunnel_pair(&mut app, (5, 5), (20, 20));

    utils::build_entity(&mut app, raiser_id, "tunnel", 25, 25);
    utils::run_ticks(&mut app, utils::APPLY + RISE_TIME + 1);

    assert_eq!(utils::owned_of_type(app.world_mut(), "tunnel", 0).len(), 2);
}

#[test]
fn second_tunnel_end_queued_while_unbound_is_born_unbound() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);

    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + 2 * TRAIN_TIME + 2);

    let mut ends: Vec<(SimulationId, Entity)> = utils::owned_of_type(app.world_mut(), "tunnel", 0)
        .into_iter()
        .map(|end| (entity_def::simulation_id(app.world(), end), end))
        .collect();
    ends.sort_unstable();
    let [_, (first_id, _), (_, second)] = ends[..] else {
        panic!("three tunnel ends, found {}", ends.len());
    };
    // Bores 1 and 2 went to the raiser and the first end, which gave its own
    // way; the second end's, 3, stands.
    assert_eq!(end_of(&app, raiser), (OtherEnd::Bound(first_id), BoreId(1)));
    assert_eq!(end_of(&app, second), (OtherEnd::Unbound, BoreId(3)));
}

#[test]
fn trench_site_lost_leaves_its_other_end_standing() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "trench", 5, 5, 0);
    utils::build_entity(&mut app, raiser_id, "trench", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let site = other_of(&mut app, "trench", raiser);
    assert!(entity_def::rising(app.world(), site));

    spawn::destroy_entity(app.world_mut(), site);

    assert!(!entity_def::dying(app.world(), raiser));
    assert_eq!(end_of(&app, raiser).0, OtherEnd::Unbound);
}

#[test]
fn tunnel_end_changing_into_shaft_keeps_its_binding_and_bore() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);
    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);
    let raised = other_of(&mut app, "tunnel", raiser);
    let raised_id = entity_def::simulation_id(app.world(), raised);

    utils::command_morph(&mut app, raiser_id, "shaft");
    utils::run_ticks(&mut app, utils::APPLY + CHANGE_TIME + 1);

    assert_eq!(entity_def::of(app.world(), raiser).name, "shaft");
    assert_eq!(
        end_of(&app, raiser),
        (OtherEnd::Bound(raised_id), BoreId(1))
    );
    assert_eq!(
        end_of(&app, raised),
        (OtherEnd::Bound(raiser_id), BoreId(1))
    );
}

//
// ─── Entering ────────────────────────────────────────────────────────────────
//

#[test]
fn beast_entering_burrow_goes_into_its_warren() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 6, 10, 0);

    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    assert!(entity_def::hidden(app.world(), beast));
    assert_eq!(held_in(&app, burrow), vec![beast_id]);
}

#[test]
fn full_warren_turns_beast_away() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    // Three fill a warren of three; the fourth is turned away.
    let beasts: Vec<_> = (0..4)
        .map(|row| utils::create_owned(&mut app, "beast", 6, 8 + row, 0))
        .collect();
    for &(_, beast_id) in &beasts {
        utils::send_to(&mut app, beast_id, burrow_id);
        utils::run_ticks(&mut app, utils::APPLY + 10);
    }

    assert_eq!(
        held_in(&app, burrow),
        beasts[..3].iter().map(|&(_, id)| id).collect::<Vec<_>>()
    );
    assert!(!entity_def::hidden(app.world(), beasts[3].0));
}

#[test]
fn beast_is_turned_away_from_other_players_burrow() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 1);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    utils::select(&mut app, beast_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: burrow_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 10);

    assert!(!entity_def::hidden(app.world(), beast));
    assert!(held_in(&app, burrow).is_empty());
}

#[test]
fn neutral_burrow_is_of_no_warren_and_turns_beast_away() {
    let mut app = app();
    let (burrow, burrow_id) =
        utils::create_entity(app.world_mut(), "burrow", utils::pos(10, 10), None)
            .expect("burrow fits at (10, 10)");
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    utils::select(&mut app, beast_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: burrow_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 10);

    assert_eq!(links::membership_of(app.world(), burrow), Membership::Apart);
    assert!(!entity_def::hidden(app.world(), beast));
}

#[test]
fn beast_crossing_tunnel_steps_out_beside_its_other_end() {
    let mut app = app();
    let ((_, raiser_id), _) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 3, 5, 0);

    utils::send_to(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    // Out on the ring around (20, 20), the cell nearest where it went in.
    assert!(!entity_def::hidden(app.world(), beast));
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(19, 19));
}

#[test]
fn tunnel_turns_beast_away_while_its_other_end_is_disabled() {
    let mut app = app();
    let ((_, raiser_id), raised) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::disable_for(&mut app, raised, 1000);

    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(4, 5));
}

#[test]
fn unbound_tunnel_turns_beast_away() {
    let mut app = app();
    let (_, tunnel_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);

    utils::send_to(&mut app, beast_id, tunnel_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(4, 5));
}

#[test]
fn burrow_turns_away_what_its_network_does_not_carry() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (critter, critter_id) = utils::create_owned(&mut app, "critter", 9, 10, 0);

    utils::board(&mut app, critter_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(!entity_def::hidden(app.world(), critter));
    assert!(held_in(&app, burrow).is_empty());
}

#[test]
fn tunnel_turns_away_what_it_does_not_carry() {
    let mut app = app();
    let ((_, raiser_id), _) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let (critter, critter_id) = utils::create_owned(&mut app, "critter", 4, 5, 0);

    utils::board(&mut app, critter_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), critter), CellPos::new(4, 5));
}

#[test]
fn exit_only_burrow_turns_beast_away() {
    let mut app = app();
    let (_, exit_id) = utils::create_owned(&mut app, "exit_burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    utils::board(&mut app, beast_id, exit_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(!entity_def::hidden(app.world(), beast));
}

#[test]
fn chute_turns_beast_away_toward_end_that_lets_nobody_out() {
    let mut app = app();
    let ((_, head_id), _) = chute(&mut app);
    // South of the head, off the ring of the foot at (5, 4).
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 5, 6, 0);

    utils::board(&mut app, beast_id, head_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(5, 6));
}

#[test]
fn beast_entering_chute_foot_steps_out_beside_its_head() {
    let mut app = app();
    let (_, foot) = chute(&mut app);
    let foot_id = entity_def::simulation_id(app.world(), foot);
    // The foot stands at (5, 4), first by row of the head's sides; the beast
    // north of it, off the head's ring.
    assert_eq!(utils::cell_of(app.world(), foot), CellPos::new(5, 4));
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 5, 3, 0);

    utils::board(&mut app, beast_id, foot_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Of the head's ring, the foot holds (5, 4), nearest where the beast
    // stood; of the two corners next nearest, (4, 4) comes first.
    assert!(!entity_def::hidden(app.world(), beast));
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(4, 4));
}

#[test]
fn beast_entering_den_goes_into_its_bore() {
    let mut app = app();
    let ((_, raiser_id), _) = den_pair(&mut app);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);

    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(entity_def::hidden(app.world(), beast));
    assert_eq!(held_in_key(&app, den_bore(&app, BoreId(1))), vec![beast_id]);
}

#[test]
fn full_den_turns_beast_away() {
    let mut app = app();
    let ((den, raiser_id), _) = den_pair(&mut app);
    // Two fill a den of two; the third is turned away.
    let beasts: Vec<_> = (0..3)
        .map(|row| utils::create_owned(&mut app, "beast", 4, 4 + row, 0))
        .collect();
    for &(_, beast_id) in &beasts {
        utils::board(&mut app, beast_id, raiser_id);
        utils::run_ticks(&mut app, utils::APPLY + 1);
    }

    assert_eq!(
        held_in(&app, den),
        beasts[..2].iter().map(|&(_, id)| id).collect::<Vec<_>>()
    );
    assert!(!entity_def::hidden(app.world(), beasts[2].0));
}

#[test]
#[should_panic(expected = "only a transporter takes a passenger aboard")]
fn boarding_burrow_directly_breaks_contract() {
    let mut app = app();
    let (burrow, _) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, _) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    spawn::board(app.world_mut(), beast, burrow);
}

//
// ─── Loading ─────────────────────────────────────────────────────────────────
//

#[test]
fn burrow_loads_beast_within_its_load_range_into_its_warren() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    utils::load(&mut app, burrow_id, beast_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    assert!(entity_def::hidden(app.world(), beast));
    assert_eq!(held_in(&app, burrow), vec![beast_id]);
}

#[test]
fn burrow_calls_beast_beyond_its_load_range_over_to_board_it() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    // Four cells off, beyond the one-cell load range of a burrow that cannot
    // walk to it.
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 14, 10, 0);

    utils::load(&mut app, burrow_id, beast_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert!(entity_def::hidden(app.world(), beast));
    assert_eq!(held_in(&app, burrow), vec![beast_id]);
}

#[test]
fn mole_takes_in_beast_part_way_across_cells_beside_it_without_walking() {
    let mut app = app();
    let projection = app.world().resource::<Map>().projection();
    utils::install_map(&mut app, projection, MovementModel::Continuous);
    let (mole, mole_id) = utils::create_owned(&mut app, "mole", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 10, 6, 0);
    // Floored into (10, 8), two cells off the mole; rounded to (10, 9), one
    // off, inside its one-cell load range.
    let corner = utils::part_way("10.4", "8.6");
    utils::select(&mut app, beast_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: corner,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::position_of(app.world_mut(), beast), corner);

    utils::load(&mut app, mole_id, beast_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    assert_eq!(held_in(&app, mole), vec![beast_id]);
    assert_eq!(
        utils::position_of(app.world_mut(), mole),
        utils::pos(10, 10)
    );
}

#[test]
fn tunnel_end_loads_beast_across_to_its_other_end() {
    let mut app = app();
    let ((_, raiser_id), _) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);

    utils::load(&mut app, raiser_id, beast_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    // Out on the ring around (20, 20), the cell nearest where it was taken in.
    assert!(!entity_def::hidden(app.world(), beast));
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(19, 19));
}

#[test]
fn tunnel_end_takes_body_seating_others_across_with_them() {
    let mut app = app();
    let ((_, raiser_id), _) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let mother = utils::place(&mut app, "brood_beast", 4, 5, 0);
    let mother_id = entity_def::simulation_id(app.world(), mother);
    // Its one larva is bred on its first tick.
    utils::run_ticks(&mut app, 1);
    let larva = utils::single_owned_of_type(app.world_mut(), "larva", 0);
    let seated = utils::offset(
        utils::position_of(app.world(), mother),
        utils::position_of(app.world(), larva),
    );

    utils::board(&mut app, mother_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    // Out on the ring around (20, 20), the larva still on its berth.
    assert_eq!(utils::cell_of(app.world(), mother), CellPos::new(19, 19));
    assert!(entity_def::attached(app.world(), larva));
    assert_eq!(
        utils::offset(
            utils::position_of(app.world(), mother),
            utils::position_of(app.world(), larva),
        ),
        seated
    );
}

#[test]
fn burrow_turns_away_body_seating_others() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (mother, mother_id) = utils::create_owned(&mut app, "brood_beast", 9, 10, 0);

    utils::board(&mut app, mother_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    assert!(!entity_def::hidden(app.world(), mother));
    assert!(held_in(&app, burrow).is_empty());
}

#[test]
fn exit_only_burrow_loads_nobody() {
    let mut app = app();
    let (_, exit_id) = utils::create_owned(&mut app, "exit_burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    utils::load(&mut app, exit_id, beast_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    assert!(!entity_def::hidden(app.world(), beast));
}

#[test]
fn mole_walks_into_its_load_range_before_taking_beast_in() {
    let mut app = app();
    let (mole, mole_id) = utils::create_owned(&mut app, "mole", 5, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 12, 10, 0);

    utils::load(&mut app, mole_id, beast_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert!(entity_def::hidden(app.world(), beast));
    assert_eq!(held_in(&app, mole), vec![beast_id]);
    assert_eq!(utils::cell_of(app.world(), mole), CellPos::new(11, 10));
}

#[test]
fn grove_loads_nobody_of_another_player_even_allied() {
    let mut app = teamed_app(0);
    let (grove, grove_id) = utils::create_owned(&mut app, "grove", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 1);

    utils::load(&mut app, grove_id, beast_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    assert!(!entity_def::hidden(app.world(), beast));
    assert!(held_in(&app, grove).is_empty());
}

#[test]
fn slow_burrow_takes_one_beast_in_every_period() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "slow_burrow", 10, 10, 0);
    let (_, first) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    let (_, second) = utils::create_owned(&mut app, "beast", 11, 10, 0);
    utils::board(&mut app, first, burrow_id);
    utils::board(&mut app, second, burrow_id);

    // Both stand beside the burrow when the orders land on tick 3: the first
    // goes in then, the second at 3 + SLOW_PERIOD = 7.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(held_in(&app, burrow), vec![first]);
    utils::run_ticks(&mut app, SLOW_PERIOD - 1);
    assert_eq!(held_in(&app, burrow), vec![first]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(held_in(&app, burrow), vec![first, second]);
}

//
// ─── Letting out ─────────────────────────────────────────────────────────────
//

#[test]
fn beast_held_at_one_burrow_is_let_out_at_another() {
    let mut app = app();
    let (_, entrance_id) = utils::create_owned(&mut app, "burrow", 5, 10, 0);
    let (exit, exit_id) = utils::create_owned(&mut app, "burrow", 25, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 10, 0);
    utils::send_to(&mut app, beast_id, entrance_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert!(entity_def::hidden(app.world(), beast));

    utils::unload(&mut app, exit_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the far burrow at (25, 10): of its ring, the four sides lie
    // nearest its middle, (25, 9) first by row.
    assert!(!entity_def::hidden(app.world(), beast));
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(25, 9));
    assert!(held_in(&app, exit).is_empty());
}

#[test]
fn burrow_lets_beast_out_at_point_within_its_unload_range() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    // Two cells off, the burrow's unload range.
    utils::unload(&mut app, burrow_id, Some(utils::pos(12, 10)));
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(12, 10));
    assert!(held_in(&app, burrow).is_empty());
}

#[test]
fn burrow_lets_beast_out_nearest_point_beyond_its_unload_range() {
    let mut app = app();
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    // Eight cells east: out at the edge of the two-cell range toward it.
    utils::unload(&mut app, burrow_id, Some(utils::pos(18, 10)));
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(12, 10));
}

#[test]
fn burrow_lets_beast_out_nearest_point_far_beyond_its_unload_range() {
    let mut app = app();
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    // Twenty cells east, past the placement search's reach from the point:
    // still out at the edge of the two-cell range toward it.
    utils::unload(&mut app, burrow_id, Some(utils::pos(30, 10)));
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(12, 10));
}

#[test]
fn burrow_lets_sapper_out_only_where_its_placement_rules_admit_it() {
    let mut app = app();
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (sapper, sapper_id) = utils::create_owned(&mut app, "sapper", 9, 10, 0);
    utils::board(&mut app, sapper_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    cover_with_creep(&mut app, CellPos::new(12, 10));

    utils::unload(&mut app, burrow_id, Some(utils::pos(12, 10)));
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Creep holds the point (12, 10); of the in-range cells around it,
    // (12, 9) comes first by row.
    assert_eq!(utils::cell_of(app.world(), sapper), CellPos::new(12, 9));
}

#[test]
fn burrow_lets_sapper_out_beside_it_only_where_its_placement_rules_admit_it() {
    let mut app = app();
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (sapper, sapper_id) = utils::create_owned(&mut app, "sapper", 9, 10, 0);
    utils::board(&mut app, sapper_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    cover_with_creep(&mut app, CellPos::new(10, 9));

    utils::unload(&mut app, burrow_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Of the burrow's four sides, nearest its middle, creep holds (10, 9),
    // first by row; (9, 10) comes next.
    assert_eq!(utils::cell_of(app.world(), sapper), CellPos::new(9, 10));
}

#[test]
fn entrance_only_burrow_lets_nobody_out() {
    let mut app = app();
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 5, 10, 0);
    let (_, entrance_id) = utils::create_owned(&mut app, "entrance_burrow", 25, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 10, 0);
    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::unload(&mut app, entrance_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(entity_def::hidden(app.world(), beast));
}

#[test]
fn beast_held_in_den_is_let_out_at_its_other_end() {
    let mut app = app();
    let ((_, raiser_id), raised) = den_pair(&mut app);
    let raised_id = entity_def::simulation_id(app.world(), raised);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::unload(&mut app, raised_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The other end stands at (5, 4), first by row of the raiser's sides;
    // of its own ring, (5, 3) is first by row of the sides.
    assert_eq!(utils::cell_of(app.world(), raised), CellPos::new(5, 4));
    assert!(!entity_def::hidden(app.world(), beast));
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(5, 3));
}

#[test]
fn slow_burrow_lets_one_beast_out_every_period() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "slow_burrow", 10, 10, 0);
    let (_, first) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    let (_, second) = utils::create_owned(&mut app, "beast", 11, 10, 0);
    for beast in [first, second] {
        utils::board(&mut app, beast, burrow_id);
        utils::run_ticks(&mut app, utils::APPLY + SLOW_PERIOD);
    }
    assert_eq!(held_in(&app, burrow), vec![first, second]);

    utils::unload(&mut app, burrow_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(held_in(&app, burrow), vec![second]);
    utils::run_ticks(&mut app, SLOW_PERIOD);
    assert!(held_in(&app, burrow).is_empty());
}

#[test]
fn grove_owner_lets_allys_beast_out_without_sending_it_on() {
    let mut app = teamed_app(0);
    let (_, grove_id) = utils::create_owned(&mut app, "grove", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, grove_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    spawn::change_owner(app.world_mut(), beast, 1, grove_id, Lean::Nothing);

    utils::unload(&mut app, grove_id, Some(utils::pos(20, 10)));
    utils::run_ticks(&mut app, utils::APPLY + 10);

    // Set down at the edge of the two-cell range toward the point, and left
    // there: an ally's unit takes no order from the grove.
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(12, 10));
}

#[test]
fn grove_sends_no_allys_beast_to_its_rally() {
    let mut app = teamed_app(0);
    let (grove, grove_id) = utils::create_owned(&mut app, "grove", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, grove_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    spawn::change_owner(app.world_mut(), beast, 1, grove_id, Lean::Nothing);
    utils::set_rally_to(
        &mut app,
        grove_id,
        RallyTarget::Position(utils::pos(20, 10)),
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(held_in(&app, grove), vec![beast_id]);

    utils::unload(&mut app, grove_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    // Set down beside the grove on the side of its rally point and left there:
    // the grove sends on only its own owner's units.
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(11, 10));
}

#[test]
fn wagon_lets_sapper_out_only_where_its_placement_rules_admit_it() {
    let mut app = app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (sapper, _) = utils::create_owned(&mut app, "sapper", 9, 10, 0);
    assert_eq!(
        spawn::board(app.world_mut(), sapper, wagon),
        Boarding::Boarded
    );
    cover_with_creep(&mut app, CellPos::new(10, 9));

    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Of the wagon's four sides, nearest its middle, creep holds (10, 9),
    // first by row; (9, 10) comes next.
    assert_eq!(utils::cell_of(app.world(), sapper), CellPos::new(9, 10));
}

#[test]
fn wagon_lets_out_beast_behind_sapper_no_cell_takes() {
    let mut app = app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    // Boarded first, the sapper goes first by id.
    let (sapper, _) = utils::create_owned(&mut app, "sapper", 9, 10, 0);
    let (beast, _) = utils::create_owned(&mut app, "beast", 11, 10, 0);
    for body in [sapper, beast] {
        assert_eq!(
            spawn::board(app.world_mut(), body, wagon),
            Boarding::Boarded
        );
    }
    // Creep ten cells every way around the wagon, past where any set-down
    // searches: nothing the sapper may stand on.
    for y in 0..=20 {
        for x in 0..=20 {
            cover_with_creep(&mut app, CellPos::new(x, y));
        }
    }

    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The beast out on the first cell of the wagon's ring, (10, 9) by row;
    // the sapper still aboard, the Unload still waiting on it.
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(10, 9));
    assert!(entity_def::hidden(app.world(), sapper));
    assert!(!entity_def::orders(app.world(), wagon).is_empty());
}

#[test]
fn burrow_lets_out_beast_behind_sapper_no_cell_takes() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    // Created first, the sapper goes first by id.
    let (sapper, sapper_id) = utils::create_owned(&mut app, "sapper", 9, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 11, 10, 0);
    utils::board(&mut app, sapper_id, burrow_id);
    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(held_in(&app, burrow), vec![sapper_id, beast_id]);
    // Creep ten cells every way around the burrow, past where any set-down
    // searches: nothing the sapper may stand on.
    for y in 0..=20 {
        for x in 0..=20 {
            cover_with_creep(&mut app, CellPos::new(x, y));
        }
    }

    utils::unload(&mut app, burrow_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The beast out on the first cell of the burrow's ring, (10, 9) by row;
    // the sapper still inside, the Unload still waiting on it.
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(10, 9));
    assert!(entity_def::hidden(app.world(), sapper));
    assert!(!entity_def::orders(app.world(), burrow).is_empty());
}

#[test]
fn own_beast_enters_at_allys_grove_and_leaves_at_own() {
    let mut app = teamed_app(0);
    let (_, allys_id) = utils::create_owned(&mut app, "grove", 10, 10, 1);
    let (own, own_id) = utils::create_owned(&mut app, "grove", 25, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    utils::board(&mut app, beast_id, allys_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(held_in(&app, own), vec![beast_id]);
    utils::unload(&mut app, own_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the own grove at (25, 10): of its ring, the four sides lie
    // nearest its middle, (25, 9) first by row.
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(25, 9));
}

#[test]
fn grove_counts_everyone_inside_against_its_capacity() {
    let mut app = teamed_app(1);
    let (grove, allys_id) = utils::create_owned(&mut app, "grove", 10, 10, 0);
    let (_, own_id) = utils::create_owned(&mut app, "grove", 20, 10, 1);
    let (_, first) = utils::create_owned(&mut app, "beast", 9, 10, 1);
    let (_, second) = utils::create_owned(&mut app, "beast", 19, 10, 1);
    let (third, third_id) = utils::create_owned(&mut app, "beast", 21, 10, 1);

    for (beast, grove_id) in [(first, allys_id), (second, own_id), (third_id, own_id)] {
        utils::board(&mut app, beast, grove_id);
        utils::run_ticks(&mut app, utils::APPLY + 1);
    }

    // Two fill a grove of two, whichever grove they went in by.
    assert_eq!(held_in(&app, grove), vec![first, second]);
    assert!(!entity_def::hidden(app.world(), third));
}

#[test]
fn beast_enters_enemys_thicket_and_leaves_at_own() {
    let mut app = teamed_app(0);
    let (_, enemys_id) = utils::create_owned(&mut app, "thicket", 10, 10, 2);
    let (_, own_id) = utils::create_owned(&mut app, "thicket", 25, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);

    utils::board(&mut app, beast_id, enemys_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::hidden(app.world(), beast));
    utils::unload(&mut app, own_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(25, 9));
}

//
// ─── Loss ────────────────────────────────────────────────────────────────────
//

#[test]
fn losing_last_burrow_destroys_everyone_in_warren() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    spawn::destroy_entity(app.world_mut(), burrow);

    assert!(entity_def::dying(app.world(), beast));
    assert!(held_in_key(&app, warren_of(&app, 0)).is_empty());
}

#[test]
fn warren_keeps_everyone_while_another_burrow_stands() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (other, other_id) = utils::create_owned(&mut app, "burrow", 25, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    spawn::destroy_entity(app.world_mut(), burrow);
    utils::run_ticks(&mut app, 2);

    assert!(!entity_def::dying(app.world(), beast));
    assert_eq!(held_in(&app, other), vec![beast_id]);
    utils::unload(&mut app, other_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!entity_def::hidden(app.world(), beast));
}

#[test]
fn capture_of_last_burrow_loses_everyone_in_old_owners_warren() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    spawn::change_owner(app.world_mut(), burrow, 1, beast_id, Lean::Nothing);

    assert!(entity_def::dying(app.world(), beast));
    assert!(held_in_key(&app, warren_of(&app, 0)).is_empty());
}

#[test]
fn captured_neutral_burrow_lets_out_everyone_in_captors_warren() {
    let mut app = app();
    let (_, entrance_id) = utils::create_owned(&mut app, "burrow", 5, 10, 0);
    let (neutral, neutral_id) =
        utils::create_entity(app.world_mut(), "burrow", utils::pos(25, 10), None)
            .expect("burrow fits at (25, 10)");
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 10, 0);
    utils::send_to(&mut app, beast_id, entrance_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    spawn::change_owner(app.world_mut(), neutral, 0, entrance_id, Lean::Nothing);
    utils::unload(&mut app, neutral_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the captured burrow at (25, 10): of its ring, the four sides lie
    // nearest its middle, (25, 9) first by row.
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(25, 9));
    assert!(held_in_key(&app, warren_of(&app, 0)).is_empty());
}

#[test]
fn tunnel_end_lost_leaves_its_other_end_standing_unbound_on_same_bore() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);
    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);
    let raised = other_of(&mut app, "tunnel", raiser);

    spawn::destroy_entity(app.world_mut(), raiser);

    assert!(!entity_def::dying(app.world(), raised));
    assert_eq!(end_of(&app, raised), (OtherEnd::Unbound, BoreId(1)));
}

#[test]
fn captured_tunnel_end_starts_on_newly_minted_bore() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);
    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);
    let raised = other_of(&mut app, "tunnel", raiser);

    spawn::change_owner(app.world_mut(), raised, 1, raiser_id, Lean::Nothing);

    // Bores 1 and 2 went to the two births; the capture mints 3.
    assert_eq!(end_of(&app, raised), (OtherEnd::Unbound, BoreId(3)));
    assert_eq!(end_of(&app, raiser), (OtherEnd::Unbound, BoreId(1)));
}

#[test]
fn trench_end_lost_razes_its_other_end() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "trench", 5, 5, 0);
    utils::train_entity(&mut app, raiser_id, "trench");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);
    let raised = other_of(&mut app, "trench", raiser);

    spawn::destroy_entity(app.world_mut(), raiser);

    assert!(entity_def::dying(app.world(), raised));
}

#[test]
fn beast_dying_inside_warren_is_struck_off_its_passengers() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    spawn::destroy_entity(app.world_mut(), beast);

    assert!(held_in(&app, burrow).is_empty());
}

#[test]
fn losing_last_hollow_sets_everyone_down_beside_it() {
    let mut app = app();
    let (hollow, hollow_id) = utils::create_owned(&mut app, "hollow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, hollow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    spawn::destroy_entity(app.world_mut(), hollow);

    // Beside the hollow at (10, 10): of its ring, the four sides lie nearest
    // its middle, (10, 9) first by row.
    assert!(!entity_def::dying(app.world(), beast));
    assert!(!entity_def::hidden(app.world(), beast));
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(10, 9));
}

#[test]
fn beast_set_down_beside_lost_hollow_faces_way_it_went_in() {
    let mut app = app();
    let (hollow, hollow_id) = utils::create_owned(&mut app, "hollow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, hollow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let went_in = app
        .world()
        .get::<HiddenComponent>(beast)
        .expect("the beast is inside the hollow")
        .went_in
        .facing;
    assert_ne!(went_in, utils::facing_of(app.world(), hollow));

    spawn::destroy_entity(app.world_mut(), hollow);

    assert_eq!(utils::facing_of(app.world(), beast), went_in);
}

#[test]
fn losing_last_hollow_sets_sapper_down_only_where_its_placement_rules_admit_it() {
    let mut app = app();
    let (hollow, hollow_id) = utils::create_owned(&mut app, "hollow", 10, 10, 0);
    let (sapper, sapper_id) = utils::create_owned(&mut app, "sapper", 9, 10, 0);
    utils::board(&mut app, sapper_id, hollow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    cover_with_creep(&mut app, CellPos::new(10, 9));

    spawn::destroy_entity(app.world_mut(), hollow);

    // Of the hollow's four sides, nearest its middle, creep holds (10, 9),
    // first by row; (9, 10) comes next.
    assert!(!entity_def::dying(app.world(), sapper));
    assert_eq!(utils::cell_of(app.world(), sapper), CellPos::new(9, 10));
}

#[test]
fn beast_lost_with_its_link_dies_at_last_burrow_lost() {
    let mut app = app();
    let (first, first_id) = utils::create_owned(&mut app, "burrow", 5, 10, 0);
    let (last, _) = utils::create_owned(&mut app, "burrow", 25, 10, 0);
    let (_, beast_id) = utils::create_owned(&mut app, "beast", 4, 10, 0);
    utils::board(&mut app, beast_id, first_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    spawn::destroy_entity(app.world_mut(), first);
    utils::run_ticks(&mut app, 2);
    utils::record_announcements(&mut app);

    spawn::destroy_entity(app.world_mut(), last);
    utils::run_ticks(&mut app, 1);

    let died_at: Vec<FixedUVec2> = app
        .world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter_map(|event| match event {
            SimulationEvent::EntityDied {
                entity, position, ..
            } if *entity == beast_id => Some(*position),
            _ => None,
        })
        .collect();
    // At the burrow whose loss settled the warren, not at (4, 10), where it
    // went in.
    assert_eq!(died_at, vec![utils::pos(25, 10)]);
}

#[test]
fn dying_wagon_sets_sapper_down_only_where_its_placement_rules_admit_it() {
    let mut app = app();
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (sapper, _) = utils::create_owned(&mut app, "sapper", 9, 10, 0);
    assert_eq!(
        spawn::board(app.world_mut(), sapper, wagon),
        Boarding::Boarded
    );
    cover_with_creep(&mut app, CellPos::new(10, 9));

    spawn::destroy_entity(app.world_mut(), wagon);

    // Of the wagon's four sides, nearest its middle, creep holds (10, 9),
    // first by row; (9, 10) comes next.
    assert!(!entity_def::dying(app.world(), sapper));
    assert_eq!(utils::cell_of(app.world(), sapper), CellPos::new(9, 10));
}

#[test]
fn beast_lost_with_link_carried_aboard_wagon_dies_where_wagon_stands() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 5, 10, 0);
    let (pocket, _) = utils::create_owned(&mut app, "pocket_burrow", 20, 10, 0);
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 21, 10, 0);
    let (_, beast_id) = utils::create_owned(&mut app, "beast", 4, 10, 0);
    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(
        spawn::board(app.world_mut(), pocket, wagon),
        Boarding::Boarded
    );
    spawn::destroy_entity(app.world_mut(), burrow);
    utils::run_ticks(&mut app, 2);
    utils::record_announcements(&mut app);

    spawn::destroy_entity(app.world_mut(), pocket);
    utils::run_ticks(&mut app, 1);

    let died_at: Vec<FixedUVec2> = app
        .world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter_map(|event| match event {
            SimulationEvent::EntityDied {
                entity, position, ..
            } if *entity == beast_id => Some(*position),
            _ => None,
        })
        .collect();
    // The last gateway rides aboard the wagon at (21, 10): the beast dies
    // there, not at (4, 10), where it went in.
    assert_eq!(died_at, vec![utils::pos(21, 10)]);
}

#[test]
fn beast_dying_aboard_dying_wagon_is_inside_nothing() {
    let mut app = app();
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (beast, _) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    assert_eq!(
        spawn::board(app.world_mut(), beast, wagon),
        Boarding::Boarded
    );

    spawn::destroy_entity(app.world_mut(), beast);
    spawn::destroy_entity(app.world_mut(), wagon);

    assert_eq!(entity_def::inside(app.world(), beast), None);
}

#[test]
fn beast_dying_aboard_wagon_is_taken_to_have_been_where_wagon_is() {
    let mut app = app();
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (beast, _) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    assert_eq!(
        spawn::board(app.world_mut(), beast, wagon),
        Boarding::Boarded
    );

    spawn::destroy_entity(app.world_mut(), beast);

    // Where the wagon stands, not (9, 10), where it boarded.
    assert_eq!(
        entity_def::whereabouts(app.world(), beast),
        utils::pos(10, 10)
    );
}

#[test]
fn passengers_lost_with_captured_last_burrow_are_owners_loss_and_nobodys_kill() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (_, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    spawn::change_owner(app.world_mut(), burrow, 1, beast_id, Lean::Nothing);
    utils::run_ticks(&mut app, 1);

    let registry = app.world().resource::<ContentRegistry>();
    let beast = registry.type_id("beast").expect("beast is registered");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(beast), 1);
    assert_eq!(statistics.player(1).killed(beast), 0);
}

#[test]
fn losing_last_hollow_kills_whoever_fits_nowhere_beside_it() {
    let mut app = app();
    let (hollow, hollow_id) = utils::create_owned(&mut app, "hollow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, hollow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);

    spawn::destroy_entity(app.world_mut(), hollow);

    assert!(entity_def::dying(app.world(), beast));
}

#[test]
fn passengers_lost_with_last_burrow_count_as_losses_and_kills() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (_, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    let (_, enemy_id) = utils::create_owned(&mut app, "beast", 20, 20, 1);
    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::despawn_killed(app.world_mut(), burrow, enemy_id, Some(1));
    utils::run_ticks(&mut app, 1);

    let registry = app.world().resource::<ContentRegistry>();
    let beast = registry.type_id("beast").expect("beast is registered");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(beast), 1);
    assert_eq!(statistics.player(1).killed(beast), 1);
}

#[test]
fn den_keeps_everyone_through_one_ends_loss() {
    let mut app = app();
    let ((raiser, raiser_id), raised) = den_pair(&mut app);
    let raised_id = entity_def::simulation_id(app.world(), raised);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    spawn::destroy_entity(app.world_mut(), raiser);
    utils::run_ticks(&mut app, 2);

    assert_eq!(held_in(&app, raised), vec![beast_id]);
    utils::unload(&mut app, raised_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!entity_def::hidden(app.world(), beast));
}

#[test]
fn losing_last_den_end_destroys_everyone_in_its_bore() {
    let mut app = app();
    let ((raiser, raiser_id), raised) = den_pair(&mut app);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    spawn::destroy_entity(app.world_mut(), raiser);
    utils::run_ticks(&mut app, 2);
    spawn::destroy_entity(app.world_mut(), raised);

    assert!(entity_def::dying(app.world(), beast));
    assert!(held_in_key(&app, den_bore(&app, BoreId(1))).is_empty());
}

#[test]
fn captured_last_thicket_keeps_everyone_in_its_shared_hold() {
    let mut app = teamed_app(0);
    let (thicket, thicket_id) = utils::create_owned(&mut app, "thicket", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, thicket_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    spawn::change_owner(app.world_mut(), thicket, 2, beast_id, Lean::Nothing);

    assert!(!entity_def::dying(app.world(), beast));
    assert_eq!(held_in(&app, thicket), vec![beast_id]);
}

#[test]
fn last_grove_captured_within_team_keeps_everyone() {
    let mut app = teamed_app(0);
    let (grove, grove_id) = utils::create_owned(&mut app, "grove", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, grove_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    spawn::change_owner(app.world_mut(), grove, 1, beast_id, Lean::Nothing);

    assert!(!entity_def::dying(app.world(), beast));
    assert_eq!(held_in(&app, grove), vec![beast_id]);
}

#[test]
fn captured_trench_end_leaves_its_other_end_standing() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "trench", 5, 5, 0);
    utils::train_entity(&mut app, raiser_id, "trench");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);
    let raised = other_of(&mut app, "trench", raiser);

    spawn::change_owner(app.world_mut(), raised, 1, raiser_id, Lean::Nothing);

    assert!(!entity_def::dying(app.world(), raiser));
    assert_eq!(end_of(&app, raiser).0, OtherEnd::Unbound);
}

//
// ─── Change of form ──────────────────────────────────────────────────────────
//

#[test]
fn last_burrow_changing_into_rock_destroys_everyone_in_warren() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    utils::command_morph(&mut app, burrow_id, "rock");
    utils::run_ticks(&mut app, utils::APPLY + CHANGE_TIME + 1);

    // Destroyed when the burrow became a rock, it died out in its one tick.
    assert_eq!(entity_def::of(app.world(), burrow).name, "rock");
    assert!(app.world().get_entity(beast).is_err());
    assert!(held_in_key(&app, warren_of(&app, 0)).is_empty());
}

#[test]
fn passengers_lost_with_last_burrow_changing_form_are_owners_loss_and_nobodys_kill() {
    let mut app = app();
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (_, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    utils::command_morph(&mut app, burrow_id, "rock");
    utils::run_ticks(&mut app, utils::APPLY + CHANGE_TIME + 1);

    let registry = app.world().resource::<ContentRegistry>();
    let beast = registry.type_id("beast").expect("beast is registered");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(beast), 1);
    assert_eq!(statistics.player(0).killed(beast), 0);
    assert_eq!(statistics.player(1).killed(beast), 0);
}

#[test]
fn burrow_changing_into_great_burrow_keeps_everyone_in_warren() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::send_to(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    utils::command_morph(&mut app, burrow_id, "great_burrow");
    utils::run_ticks(&mut app, utils::APPLY + CHANGE_TIME + 1);

    assert_eq!(entity_def::of(app.world(), burrow).name, "great_burrow");
    assert!(!entity_def::dying(app.world(), beast));
    assert_eq!(held_in(&app, burrow), vec![beast_id]);
}

#[test]
fn rock_changing_into_burrow_lets_out_everyone_in_owners_warren() {
    let mut app = app();
    let (_, entrance_id) = utils::create_owned(&mut app, "burrow", 5, 10, 0);
    let (_, rock_id) = utils::create_owned(&mut app, "rock", 25, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 10, 0);
    utils::send_to(&mut app, beast_id, entrance_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    utils::command_morph(&mut app, rock_id, "burrow");
    utils::run_ticks(&mut app, utils::APPLY + CHANGE_TIME + 1);
    utils::unload(&mut app, rock_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Beside the new burrow at (25, 10): of its ring, the four sides lie
    // nearest its middle, (25, 9) first by row.
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(25, 9));
}

#[test]
fn burrow_changing_straight_into_rock_turns_beast_away() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::command_morph(&mut app, burrow_id, "rock");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Two ticks into a ten-tick change, still a burrow.
    assert_eq!(entity_def::of(app.world(), burrow).name, "burrow");
    assert!(!entity_def::hidden(app.world(), beast));
    assert!(held_in(&app, burrow).is_empty());
}

#[test]
fn burrow_deepening_lets_beast_into_warren_mid_change() {
    let mut app = app();
    let (burrow, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::command_morph(&mut app, burrow_id, "deep_burrow");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::board(&mut app, beast_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(entity_def::of(app.world(), burrow).name, "burrow_deepening");
    assert_eq!(held_in(&app, burrow), vec![beast_id]);

    utils::run_ticks(&mut app, CHANGE_TIME);

    assert_eq!(entity_def::of(app.world(), burrow).name, "deep_burrow");
    assert!(entity_def::hidden(app.world(), beast));
    assert_eq!(held_in(&app, burrow), vec![beast_id]);
}

#[test]
fn tunnel_turns_beast_away_while_its_other_end_changes_straight_into_trench_end() {
    let mut app = app();
    let ((_, raiser_id), raised) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let raised_id = entity_def::simulation_id(app.world(), raised);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::command_morph(&mut app, raised_id, "trench");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(entity_def::of(app.world(), raised).name, "tunnel");
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(4, 5));
}

#[test]
fn tunnel_end_changing_into_trench_end_leaves_its_other_end_and_starts_on_newly_minted_bore() {
    let mut app = app();
    let (raiser, raiser_id) = utils::create_owned(&mut app, "tunnel", 5, 5, 0);
    utils::train_entity(&mut app, raiser_id, "tunnel");
    utils::run_ticks(&mut app, utils::APPLY + TRAIN_TIME + 1);
    let raised = other_of(&mut app, "tunnel", raiser);

    utils::command_morph(&mut app, raiser_id, "trench");
    utils::run_ticks(&mut app, utils::APPLY + CHANGE_TIME + 1);

    // Bores 1 and 2 went to the two births; the new form mints 3.
    assert_eq!(end_of(&app, raiser), (OtherEnd::Unbound, BoreId(3)));
    assert_eq!(end_of(&app, raised), (OtherEnd::Unbound, BoreId(1)));
}

//
// ─── Crossing ────────────────────────────────────────────────────────────────
//

#[test]
fn beast_crossing_tunnel_is_announced_relocated() {
    let mut app = app();
    let ((_, raiser_id), _) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let (_, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::record_announcements(&mut app);

    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(app.world().resource::<utils::Announced>().0.contains(
        &SimulationEvent::EntityRelocated {
            entity: beast_id,
            from: utils::pos(4, 5),
            to: utils::pos(19, 19),
        }
    ));
}

#[test]
fn beast_crossing_tunnel_is_sent_on_to_other_ends_rally() {
    let mut app = app();
    let ((_, raiser_id), raised) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let raised_id = entity_def::simulation_id(app.world(), raised);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::set_rally_to(
        &mut app,
        raised_id,
        RallyTarget::Position(utils::pos(25, 20)),
    );

    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 30);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(25, 20));
}

#[test]
fn beast_crossing_tunnel_follows_walk_queued_behind_its_board_over_other_ends_rally() {
    let mut app = app();
    let ((_, raiser_id), raised) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let raised_id = entity_def::simulation_id(app.world(), raised);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::set_rally_to(
        &mut app,
        raised_id,
        RallyTarget::Position(utils::pos(25, 20)),
    );

    utils::board(&mut app, beast_id, raiser_id);
    queue_move(&mut app, 15, 25);
    utils::run_ticks(&mut app, utils::APPLY + 40);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(15, 25));
}

#[test]
fn beast_entering_burrow_drops_walk_queued_behind_its_board() {
    let mut app = app();
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 10, 10, 0);
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 9, 10, 0);
    utils::board(&mut app, beast_id, burrow_id);
    queue_move(&mut app, 20, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    utils::unload(&mut app, burrow_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 30);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(10, 9));
}

#[test]
fn rally_naming_either_end_of_crossed_tunnel_sends_nobody() {
    for named in [Named::Raiser, Named::Raised] {
        let mut app = app();
        let ((raiser, raiser_id), raised) = tunnel_pair(&mut app, (5, 5), (20, 20));
        let raised_id = entity_def::simulation_id(app.world(), raised);
        let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
        let target = match named {
            Named::Raiser => entity_def::simulation_id(app.world(), raiser),
            Named::Raised => raised_id,
        };
        utils::set_rally_to(&mut app, raised_id, RallyTarget::Entity(target));

        utils::board(&mut app, beast_id, raiser_id);
        utils::run_ticks(&mut app, utils::APPLY + 30);

        assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(19, 19));
    }
}

#[test]
fn beast_waits_to_cross_until_cell_beside_other_end_frees() {
    let mut app = app();
    let ((_, raiser_id), _) = tunnel_pair(&mut app, (5, 5), (20, 20));
    let (beast, beast_id) = utils::create_owned(&mut app, "beast", 4, 5, 0);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);

    utils::board(&mut app, beast_id, raiser_id);
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(4, 5));
    utils::free_cell(&mut app, CellPos::new(21, 21));
    utils::run_ticks(&mut app, 2);

    assert_eq!(utils::cell_of(app.world(), beast), CellPos::new(21, 21));
}

//
// ─── Brain's view ────────────────────────────────────────────────────────────
//

#[test]
fn own_burrow_lists_everyone_in_its_warren_to_brain() {
    let mut app = app();
    let (_, entrance_id) = utils::create_owned(&mut app, "burrow", 5, 10, 0);
    let (_, exit_id) = utils::create_owned(&mut app, "burrow", 25, 10, 0);
    let (_, beast_id) = utils::create_owned(&mut app, "beast", 4, 10, 0);
    utils::send_to(&mut app, beast_id, entrance_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    let view = game_view(
        app.world_mut(),
        0,
        "beasts",
        AiVision::Filtered,
        AiDetection::Detectors,
    );

    // Either burrow lets it out, so both list it.
    for burrow_id in [entrance_id, exit_id] {
        let burrow = view
            .my_entities
            .iter()
            .find(|entity| entity.id == burrow_id.0)
            .expect("a side keeps its own in view");
        assert_eq!(burrow.passengers, vec![beast_id.0]);
    }
}

#[test]
fn thicket_lists_own_and_allied_passengers_to_brain_but_not_enemys() {
    let mut app = teamed_app(0);
    let (_, thicket_id) = utils::create_owned(&mut app, "thicket", 10, 10, 0);
    let beasts: Vec<_> = (0..3)
        .map(|row| utils::create_owned(&mut app, "beast", 9, 9 + row, 0))
        .collect();
    for &(_, beast_id) in &beasts {
        utils::board(&mut app, beast_id, thicket_id);
        utils::run_ticks(&mut app, utils::APPLY + 1);
    }
    spawn::change_owner(app.world_mut(), beasts[1].0, 1, thicket_id, Lean::Nothing);
    spawn::change_owner(app.world_mut(), beasts[2].0, 2, thicket_id, Lean::Nothing);

    let view = game_view(
        app.world_mut(),
        0,
        "beasts",
        AiVision::Filtered,
        AiDetection::Detectors,
    );

    let thicket = view
        .my_entities
        .iter()
        .find(|entity| entity.id == thicket_id.0)
        .expect("a side keeps its own in view");
    assert_eq!(thicket.passengers, vec![beasts[0].1.0, beasts[1].1.0]);
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Ticks a `tunnel` end takes to train another.
const TRAIN_TIME: u32 = 5;

/// Ticks of work a `tunnel` end takes to rise.
const RISE_TIME: u32 = 5;

/// Ticks every change of form in the fixture takes.
const CHANGE_TIME: u32 = 10;

/// Ticks a `slow_burrow` waits between one body and the next.
const SLOW_PERIOD: u32 = 4;

/// App with two players apart, commanded as player 0 (see [`register`]).
fn app() -> App {
    seated_app(utils::human_slots(2), 0)
}

/// App with players 0 and 1 on one team and player 2 on none, commanded as
/// `local` (see [`register`]).
fn teamed_app(local: PlayerId) -> App {
    let slots = [Some(0), Some(0), None]
        .into_iter()
        .enumerate()
        .map(|(id, team)| PlayerSlot::occupied(id as PlayerId, PlayerType::Human, None, team))
        .collect();
    seated_app(slots, local)
}

/// App seated on `slots`, commanded as `local`, the fixture registered.
fn seated_app(slots: Vec<PlayerSlot>, local: PlayerId) -> App {
    let mut app = utils::make_app_as(slots, local);
    register(&mut app);
    app.world_mut().resource::<ContentRegistry>().validate();
    utils::start_game(&mut app);
    app
}

/// Registers the fixture, every gateway one cell and both ways unless named
/// otherwise, every one taking bodies in from one cell off and letting them
/// out up to two cells off, at once:
/// - a `warren` network of three own `beast`s that dies with its last
///   gateway: `burrow`, `great_burrow`, `burrow_deepening`, `deep_burrow`,
///   the entrance-only `entrance_burrow`, the exit-only `exit_burrow`, a
///   `slow_burrow` taking one body in and letting one out every
///   `SLOW_PERIOD` ticks, a walking `mole`, and a walking exit-only
///   `pocket_burrow` that rides, as `portable`;
/// - a `hollow` network of three own `beast`s set down beside its last
///   gateway, the `hollow`;
/// - a `grove` network of two allied `beast`s, the `grove`, and a `thicket`
///   network of three anyone's, the `thicket`;
/// - a `tunnel` crossing at once whose end stands when its other end is lost:
///   the `tunnel` end training and founding another, and the `shaft` end,
///   seeing five cells around, summoning a `tunnel` end with `dig` and
///   raising one from a `corpse` with `exhume`;
/// - a `trench` crossing at once whose end is razed with its other end, each
///   end training and founding another;
/// - a one-way `chute`: the `chute_head` end training a `chute_foot`, the
///   foot an entrance only;
/// - a `den` tunnel holding two own `beast`s inside, killed with its last
///   end, each `den` end training another;
/// - a walking `wagon` carrying beasts and portables, setting them down when
///   it dies;
/// - a `rock` that is no gateway, a walking `beast` that leaves a `corpse` and
///   sees three cells around, a walking `sapper` beast that never stands on
///   lasting `creep`, a walking `brood_beast` born seating a `larva` in its
///   one berth, and a walking `critter` that is no beast.
///
/// A `burrow` changes into a `rock`, a `great_burrow`, and a `deep_burrow`
/// through `burrow_deepening`; a `rock` into a `burrow`; a `tunnel` end into
/// a `trench` end or a `shaft` end. Every change takes `CHANGE_TIME` ticks.
fn register(app: &mut App) {
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    registry.register_tag("beast");
    registry.register_tag("critter");
    registry.register_tag("portable");
    let beasts = || Kinds::tags(["beast"]);
    let network = |registry: &mut ContentRegistry,
                   name: &str,
                   boarding: Affiliation,
                   capacity: u32,
                   fate: PassengerFate| {
        registry.register_network(name, NetworkDef::new(beasts(), boarding, capacity, fate))
    };
    let warren = network(
        &mut registry,
        "warren",
        Affiliation::Own,
        3,
        PassengerFate::Destroy,
    );
    let hollow = network(
        &mut registry,
        "hollow",
        Affiliation::Own,
        3,
        PassengerFate::Eject,
    );
    let grove = network(
        &mut registry,
        "grove",
        Affiliation::Allied,
        2,
        PassengerFate::Destroy,
    );
    let thicket = network(
        &mut registry,
        "thicket",
        Affiliation::Anyone,
        3,
        PassengerFate::Destroy,
    );
    let tunnel_of = |registry: &mut ContentRegistry, name: &str, lost, passage| {
        registry.register_tunnel(
            name,
            TunnelDef::new(beasts(), Affiliation::Own, lost, passage),
        )
    };
    let tunnel = tunnel_of(
        &mut registry,
        "tunnel",
        OtherEndLost::Stands,
        Passage::Crosses,
    );
    let trench = tunnel_of(
        &mut registry,
        "trench",
        OtherEndLost::Razed,
        Passage::Crosses,
    );
    let chute = tunnel_of(
        &mut registry,
        "chute",
        OtherEndLost::Stands,
        Passage::Crosses,
    );
    let den = tunnel_of(
        &mut registry,
        "den",
        OtherEndLost::Stands,
        Passage::Held {
            capacity: 2,
            fate: PassengerFate::Destroy,
        },
    );

    for name in ["burrow", "great_burrow", "burrow_deepening", "deep_burrow"] {
        registry.register(
            leaving(entering(
                utils::building(name).with_gateway(Joins::Network(warren), Doors::Both),
            ))
            .with_morphs(match name {
                "burrow" => vec![
                    change("rock"),
                    change("great_burrow"),
                    change_through("burrow_deepening", "deep_burrow"),
                ],
                _ => Vec::new(),
            }),
        );
    }
    registry.register(entering(
        utils::building("entrance_burrow").with_gateway(Joins::Network(warren), Doors::Entrance),
    ));
    registry.register(leaving(
        utils::building("exit_burrow").with_gateway(Joins::Network(warren), Doors::Exit),
    ));
    registry.register(
        utils::building("slow_burrow")
            .with_gateway(Joins::Network(warren), Doors::Both)
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::from_num(SLOW_PERIOD))
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::from_num(2))
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::from_num(SLOW_PERIOD)),
    );
    registry.register(leaving(entering(
        utils::walker("mole", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_dying(1, [])
            .with_gateway(Joins::Network(warren), Doors::Both),
    )));
    registry.register(leaving(
        utils::walker("pocket_burrow", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_dying(1, [])
            .with_tags(["portable"])
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
            .with_gateway(Joins::Network(warren), Doors::Exit),
    ));
    registry.register(
        utils::walker("wagon", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(60))
            .with_dying(1, [])
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                Kinds::tags(["beast", "portable"]),
                Affiliation::Own,
                PassengerFate::Eject,
                PassengerConduct::Shelter,
            ),
    );
    registry.register(utils::building("rock").with_morphs([change("burrow")]));
    for (name, joins) in [
        ("hollow", Joins::Network(hollow)),
        ("grove", Joins::Network(grove)),
        ("thicket", Joins::Network(thicket)),
    ] {
        registry.register(leaving(entering(
            utils::building(name).with_gateway(joins, Doors::Both),
        )));
    }

    registry.register(
        entering(utils::building("tunnel").with_gateway(Joins::Tunnel(tunnel), Doors::Both))
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(40))
            .with_train_time(TRAIN_TIME)
            .with_trainer(["tunnel"])
            .with_rise(RISE_TIME, utils::rise_initial_pools(&[PoolId::HEALTH]))
            .with_builder(["tunnel"], BuilderAttendance::Unattended)
            .with_morphs([change("trench"), change("shaft")]),
    );
    // A tunnel end that warps its other end in, anywhere.
    registry.register(
        entering(utils::building("rift").with_gateway(Joins::Tunnel(tunnel), Doors::Both))
            .with_warper(["tunnel"], Vec::new(), Landing::Anywhere),
    );
    registry.register(
        EntityTypeDef::new("corpse")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Passable,
                utils::GROUND,
            )
            .with_tags(["remains"])
            .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
    );
    let raised = registry
        .type_id("tunnel")
        .expect("the tunnel end is registered");
    let summoning = |at| SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            casting: Casting::Instant,
            effect: EntityCastEffect::Summon {
                at,
                entity_type: raised,
                count: Quantity::Constant(1),
                landing: Landing::Anywhere,
                within: Vec::new(),
                arrives: SummonArrival::Standing,
            },
        },
        requires: Vec::new(),
    };
    let dig = registry.register_skill(
        "dig",
        summoning(SummonAt::Cell {
            reach: Reach::Wherever,
        }),
    );
    let exhume = registry.register_skill(
        "exhume",
        summoning(SummonAt::Fallen {
            kinds: Kinds::Any,
            reach: Reach::Wherever,
        }),
    );
    registry.register(
        entering(utils::building("shaft").with_gateway(Joins::Tunnel(tunnel), Doors::Both))
            .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(5))
            .with_skills([dig, exhume]),
    );
    registry.register(
        entering(utils::building("trench").with_gateway(Joins::Tunnel(trench), Doors::Both))
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(40))
            .with_train_time(TRAIN_TIME)
            .with_trainer(["trench"])
            .with_rise(RISE_TIME, utils::rise_initial_pools(&[PoolId::HEALTH]))
            .with_builder(["trench"], BuilderAttendance::Unattended),
    );
    registry.register(
        entering(utils::building("chute_head").with_gateway(Joins::Tunnel(chute), Doors::Both))
            .with_train_time(TRAIN_TIME)
            .with_trainer(["chute_foot"]),
    );
    registry.register(
        entering(utils::building("chute_foot").with_gateway(Joins::Tunnel(chute), Doors::Entrance))
            .with_train_time(TRAIN_TIME),
    );
    registry.register(
        leaving(entering(
            utils::building("den").with_gateway(Joins::Tunnel(den), Doors::Both),
        ))
        .with_train_time(TRAIN_TIME)
        .with_trainer(["den"]),
    );

    registry.register(
        utils::walker("beast", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_dying(1, utils::leaves("corpse"))
            .with_tags(["beast"])
            .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(3))
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
    registry.register(
        utils::walker("brood_beast", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_dying(1, [])
            .with_tags(["beast"])
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
            .with_berths([("brood", BerthGroup::new([utils::berth("0.5", "1.5")], 1))])
            .with_breeder("larva", Quantity::Constant(1000), 1, 1, OrphanFate::Perish),
    );
    registry.register(
        EntityTypeDef::new("larva")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Passable,
                utils::GROUND,
            )
            .with_pool(Pool::health(5))
            .with_dying(1, [])
            .with_broodling(Attachment::new("brood", BerthStance::Still)),
    );
    let creep = registry.register_field(
        "creep",
        FieldDef::new(
            Terrains::Only(utils::LAND.into()),
            utils::GROUND.into(),
            FieldDecay::Never,
            FieldVision::Dark,
            Detection::Blind,
        ),
    );
    registry.register(
        utils::walker("sapper", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_dying(1, [])
            .with_tags(["beast"])
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
            .with_field_placement([FieldPlacement::Forbids { field: creep }]),
    );
    registry.register(
        utils::walker("critter", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_dying(1, [])
            .with_tags(["critter"])
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
}

/// `def` taking bodies in from one cell off, at once.
fn entering(def: EntityTypeDef) -> EntityTypeDef {
    def.with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
        .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
}

/// `def` letting bodies out up to two cells off, at once.
fn leaving(def: EntityTypeDef) -> EntityTypeDef {
    def.with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::from_num(2))
        .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
}

/// A `CHANGE_TIME`-tick change of form into `into`, costing nothing.
fn change(into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::direct(MorphInterrupted::Reverts),
        Quantity::Constant(CHANGE_TIME),
        MorphPlacement::Reserve,
        MorphCancel::Refundable,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
    )
}

/// A `CHANGE_TIME`-tick change of form into `into` through the interim form
/// `via`, costing nothing.
fn change_through(via: &str, into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::via(
            via,
            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            ViaInterrupted::reverts([(
                PoolId::HEALTH,
                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
            )]),
        ),
        Quantity::Constant(CHANGE_TIME),
        MorphPlacement::Reserve,
        MorphCancel::Refundable,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
    )
}

/// The one `type_name` of player 0's other than `end`.
fn other_of(app: &mut App, type_name: &str, end: Entity) -> Entity {
    let others: Vec<Entity> = utils::owned_of_type(app.world_mut(), type_name, 0)
        .into_iter()
        .filter(|&other| other != end)
        .collect();
    match others.as_slice() {
        [other] => *other,
        _ => panic!("one other {type_name}, found {}", others.len()),
    }
}

/// Player 0's `tunnel` end at `at` and, finished, the other end it founds at
/// `other_at`.
fn tunnel_pair(
    app: &mut App,
    at: (u32, u32),
    other_at: (u32, u32),
) -> ((Entity, SimulationId), Entity) {
    let (raiser, raiser_id) = utils::create_owned(app, "tunnel", at.0, at.1, 0);
    utils::build_entity(app, raiser_id, "tunnel", other_at.0, other_at.1);
    utils::run_ticks(app, utils::APPLY + RISE_TIME + 1);
    ((raiser, raiser_id), other_of(app, "tunnel", raiser))
}

/// Player 0's `chute_head` at (5, 5) and the `chute_foot` it trained.
fn chute(app: &mut App) -> ((Entity, SimulationId), Entity) {
    let (head, head_id) = utils::create_owned(app, "chute_head", 5, 5, 0);
    utils::train_entity(app, head_id, "chute_foot");
    utils::run_ticks(app, utils::APPLY + TRAIN_TIME + 1);
    let foot = utils::single_owned_of_type(app.world_mut(), "chute_foot", 0);
    ((head, head_id), foot)
}

/// Player 0's `den` end at (5, 5) and the other end it trained.
fn den_pair(app: &mut App) -> ((Entity, SimulationId), Entity) {
    let (raiser, raiser_id) = utils::create_owned(app, "den", 5, 5, 0);
    utils::train_entity(app, raiser_id, "den");
    utils::run_ticks(app, utils::APPLY + TRAIN_TIME + 1);
    ((raiser, raiser_id), other_of(app, "den", raiser))
}

/// The `den` link on `bore`.
fn den_bore(app: &App, bore: BoreId) -> LinkKey {
    LinkKey::Tunnel {
        tunnel: app
            .world()
            .resource::<ContentRegistry>()
            .tunnel("den")
            .expect("den is registered"),
        bore,
    }
}

/// The `corpse` a `beast` of player 1's leaves, killed at `(x, y)` by a
/// `beast` of player 0's standing at (0, 0).
fn corpse_at(app: &mut App, x: u32, y: u32) -> SimulationId {
    let (beast, _) = utils::create_owned(app, "beast", x, y, 1);
    let (_, killer_id) = utils::create_owned(app, "beast", 0, 0, 0);
    utils::despawn_killed(app.world_mut(), beast, killer_id, Some(0));
    utils::run_ticks(app, 4);
    app.world()
        .resource::<EntityIndex>()
        .remains_entries()
        .last()
        .expect("the beast left a corpse")
        .0
}

/// Queues a walk to `(x, y)` behind what the selection is doing.
fn queue_move(app: &mut App, x: u32, y: u32) {
    utils::push_command(
        app,
        PlayerCommand::Move {
            target: utils::pos(x, y),
            flush: false,
        },
    );
}

/// Covers `cell` with player 1's `creep`.
fn cover_with_creep(app: &mut App, cell: CellPos) {
    let creep = app
        .world()
        .resource::<ContentRegistry>()
        .field("creep")
        .expect("creep is registered");
    fields::apply_action(app.world_mut(), 1, creep, cell, 0, FieldAction::Cover);
}

/// Which end of a tunnel a rally names.
#[derive(Clone, Copy)]
enum Named {
    /// The end crossed from.
    Raiser,
    /// The end crossed to.
    Raised,
}

/// Player `player`'s running `warren`.
fn warren_of(app: &App, player: PlayerId) -> LinkKey {
    LinkKey::Network {
        network: app
            .world()
            .resource::<ContentRegistry>()
            .network("warren")
            .expect("warren is registered"),
        scope: NetworkScope::Player(player),
    }
}

/// Everyone inside the link `key`, in id order.
fn held_in_key(app: &App, key: LinkKey) -> Vec<SimulationId> {
    app.world()
        .resource::<LinkPassengers>()
        .passengers(key)
        .collect()
}

/// Everyone inside the link `gateway` is one of, in id order.
fn held_in(app: &App, gateway: Entity) -> Vec<SimulationId> {
    links::passengers_of(app.world(), gateway).collect()
}

/// `end`'s binding: its other end and its bore.
fn end_of(app: &App, end: Entity) -> (OtherEnd, BoreId) {
    let end = app
        .world()
        .get::<TunnelEndComponent>(end)
        .expect("a tunnel end keeps its binding");
    (end.other, end.bore)
}
