//! Concealment and detection end-to-end: sighting, detection, concealing and
//! disabling buffs with their upkeep and interruptions, and orders lapsing
//! with sight.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{ai::game_view, map};
use ferrets_content::{
    affiliation::Affiliation,
    attack::{AttackDef, Delivery, Slain, Weapon},
    concealment::Concealment,
    cost::Cost,
    detection::Detection,
    entity_buffs::{EntityBuffDef, Interruption, Lasting},
    entity_effect::EntityEffect,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        Emission, FieldCoverage, FieldDecay, FieldDef, FieldEffect, FieldGrowth, FieldLayer,
        FieldSide, FieldSourceDef, FieldVision,
    },
    kinds::Kinds,
    location::Solidity,
    morph::{MorphCancel, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition},
    price,
    projectile::{Aim, ProjectileDef},
    quantity::Quantity,
    registry::ContentRegistry,
    skills::{Casting, EntityCastEffect, EntityCastTarget, Reach, SkillCaster, SkillDef},
    splash::{SplashDef, SplashShape},
    stack_rule::StackRule,
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_mask::LayerMask, mover_shape::MoverShape, nav_grid::NavGrid};
use ferrets_script::ai::view::game::Glimpse;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode, SkillCasterRef, SkillTarget},
    components::{
        concealed::ConcealedComponent, dying::DyingComponent, follow::FollowComponent,
        guard::GuardComponent,
    },
    entity_def::{self, Operation, Outage},
    events::{SimulationEvent, SpendCause},
    game_loop::stats,
    map::Map,
    movement_model::MovementModel,
    order::AttackTarget,
    session::{
        GameSession, ai_detection::AiDetection, ai_vision::AiVision, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    spawn,
    visibility::{self, Senses, Sighting},
};

//
// ─── Sighting ────────────────────────────────────────────────────────────────
//

#[test]
fn concealed_enemy_on_lit_cell_is_glimpsed_and_not_named() {
    let mut app = concealment_app(rivals());
    let (sniper, sniper_id) = utils::create_owned(&mut app, "sniper", 5, 5, 0);
    let (shade, shade_id) = utils::create_owned(&mut app, "shade", 5, 8, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    // The sniper sees eight cells; the shade stands three away, on a lit
    // cell, and is a presence and no more.
    assert_eq!(sighting_of(&app, 0, shade), Sighting::Glimpsed);

    // Neither the acquire scan nor a named attack reaches it.
    utils::select(&mut app, sniper_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(shade_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);
    assert!(utils::order_queue_is_empty(app.world_mut(), sniper));
    assert_eq!(utils::health(&app, shade), 20, "the shade is untouched");
}

#[test]
fn detector_in_range_lets_side_engage_concealed_enemy() {
    let mut app = concealment_app(rivals());
    utils::create_owned(&mut app, "sniper", 5, 5, 0);
    utils::create_owned(&mut app, "tower", 7, 7, 0);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 8, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(sighting_of(&app, 0, shade), Sighting::Seen);
    // Auto-engaged: 10 damage every 2 ticks empties 20 health.
    utils::run_ticks(&mut app, 30);
    utils::assert_despawned(app.world_mut(), shade);
}

#[test]
fn allied_detector_counts() {
    let mut app = concealment_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(2)),
        PlayerSlot::occupied(2, PlayerType::Human, None, Some(1)),
    ]);
    utils::create_owned(&mut app, "sniper", 5, 5, 0);
    // The ally's tower, not the sniper's owner's.
    utils::create_owned(&mut app, "tower", 7, 7, 2);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 8, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(sighting_of(&app, 0, shade), Sighting::Seen);
    utils::run_ticks(&mut app, 30);
    utils::assert_despawned(app.world_mut(), shade);
}

#[test]
fn detection_without_sight_reveals_nothing() {
    let mut app = concealment_app(rivals());
    // The tower detects to six but sees only two: the shade stands under its
    // detection and outside its sight, and nobody else looks.
    utils::create_owned(&mut app, "tower", 5, 5, 0);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 10, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 0, shade), Sighting::Unseen);

    // A scout lends the sight; detection was there all along.
    utils::create_owned(&mut app, "scout", 5, 12, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 0, shade), Sighting::Seen);
}

#[test]
fn owner_and_ally_always_see_own_concealed_unit() {
    let mut app = concealment_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
    ]);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 5, 0);
    utils::create_owned(&mut app, "scout", 5, 7, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(sighting_of(&app, 0, shade), Sighting::Seen);
    assert_eq!(sighting_of(&app, 1, shade), Sighting::Seen);
    let view = game_view(
        app.world(),
        0,
        "test",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    assert!(
        view.my_entities.iter().any(|e| e.concealed),
        "the brain knows its shade hides"
    );
}

#[test]
fn filtered_brain_receives_glimpse_positions_only() {
    let mut app = concealment_app(rivals());
    utils::create_owned(&mut app, "scout", 5, 5, 0);
    utils::create_owned(&mut app, "shade", 5, 8, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    let filtered = game_view(
        app.world(),
        0,
        "test",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    assert!(
        filtered.enemy_entities.is_empty(),
        "a glimpse names no entity"
    );
    // A glimpse is the ground it stands on and nothing else.
    assert_eq!(
        filtered.glimpses,
        vec![Glimpse {
            x: 5,
            y: 8,
            width: 1,
            height: 1,
        }]
    );

    // Omniscience lifts the fog and nothing else: the cloak still needs
    // detection, so the shade is glimpsed, not named.
    let omniscient = game_view(
        app.world(),
        0,
        "test",
        AiVision::Omniscient,
        AiDetection::Detectors,
    );
    assert!(omniscient.enemy_entities.is_empty());
    assert_eq!(
        omniscient.glimpses,
        vec![Glimpse {
            x: 5,
            y: 8,
            width: 1,
            height: 1,
        }]
    );

    // Detection everywhere names it, fog or no fog.
    let piercing = game_view(
        app.world(),
        0,
        "test",
        AiVision::Filtered,
        AiDetection::Everywhere,
    );
    assert_eq!(piercing.enemy_entities.len(), 1);
    assert!(piercing.enemy_entities[0].concealed);
    assert!(piercing.glimpses.is_empty());
}

#[test]
fn omniscient_seat_glimpses_undetected_cloak_anywhere_and_names_it_not() {
    let mut app = concealment_app(vec![
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
    ]);
    let (sniper, sniper_id) = utils::create_owned(&mut app, "sniper", 5, 5, 1);
    // Far beyond the sniper's sight: only omniscience reaches the cell at all.
    let (shade, shade_id) = utils::create_owned(&mut app, "shade", 25, 25, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 1, shade), Sighting::Glimpsed);

    // The seat may name what fog hides, but not what a cloak hides.
    utils::run_ticks_commanding(
        &mut app,
        10,
        1,
        utils::APPLY + 3,
        vec![
            PlayerCommand::SelectById {
                id: sniper_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Attack {
                target: AttackTarget::Entity(shade_id),
                flush: true,
            },
        ],
    );
    assert!(utils::order_queue_is_empty(app.world_mut(), sniper));
    assert_eq!(utils::health(&app, shade), 20);
}

#[test]
fn everywhere_detecting_seat_names_undetected_cloak_and_its_order_proceeds() {
    let mut app = concealment_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Everywhere,
            },
            None,
            None,
        ),
    ]);
    let (_, sniper_id) = utils::create_owned(&mut app, "sniper", 5, 5, 1);
    let (shade, shade_id) = utils::create_owned(&mut app, "shade", 5, 8, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 1, shade), Sighting::Seen);

    // Named, and the attack runs to the kill: 10 damage every 2 ticks over
    // 20 health.
    utils::run_ticks_commanding(
        &mut app,
        30,
        1,
        utils::APPLY + 3,
        vec![
            PlayerCommand::SelectById {
                id: sniper_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Attack {
                target: AttackTarget::Entity(shade_id),
                flush: true,
            },
        ],
    );
    utils::assert_despawned(app.world_mut(), shade);
}

#[test]
fn everywhere_detecting_seat_units_still_need_detector_to_engage_on_their_own() {
    let mut app = concealment_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Omniscient,
                detection: AiDetection::Everywhere,
            },
            None,
            None,
        ),
    ]);
    utils::create_owned(&mut app, "sniper", 5, 5, 1);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 8, 0);
    // The seat sees everything; its sniper, left to itself, fights as any
    // unit does and never picks out what no detector shows it.
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::health(&app, shade), 20);
}

#[test]
fn detection_naming_ground_leaves_cloaked_flier_glimpsed() {
    let mut app = concealment_app(rivals());
    utils::create_owned(&mut app, "ground_eye", 5, 5, 0);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 8, 1);
    let (phantom, _) = utils::create_owned(&mut app, "phantom", 8, 5, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(
        sighting_of(&app, 0, shade),
        Sighting::Seen,
        "on the ground the eye names"
    );
    assert_eq!(
        sighting_of(&app, 0, phantom),
        Sighting::Glimpsed,
        "in the air it does not"
    );
}

#[test]
fn disabled_detector_detects_nothing() {
    let mut app = concealment_app(rivals());
    // The rival's tower names the local shade beside it, until the local
    // arbiter's veil over the tower switches it off and its patch halts.
    let (tower, _) = utils::create_owned(&mut app, "tower", 10, 10, 1);
    let (shade, _) = utils::create_owned(&mut app, "shade", 12, 10, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 1, shade), Sighting::Seen);

    utils::create_owned(&mut app, "arbiter", 10, 10, 0);
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        entity_def::operation(app.world(), tower),
        Operation::Disabled(Outage::Field)
    );
    assert_eq!(sighting_of(&app, 1, shade), Sighting::Glimpsed);
}

#[test]
fn watch_detects_concealed_enemy_for_its_duration() {
    let mut app = concealment_app(rivals());
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (shade, _) = utils::create_owned(&mut app, "shade", 8, 5, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 0, shade), Sighting::Glimpsed);

    // A sweep over the shade's cell names it from the tick the cast lands
    // through the six ticks after, then the patch lapses and it is a shimmer
    // again.
    utils::use_skill(
        &mut app,
        "sweep",
        SkillCasterRef::Entity(mage_id),
        Some(SkillTarget::Position(utils::pos(8, 5))),
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 0, shade), Sighting::Seen);
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        sighting_of(&app, 0, shade),
        Sighting::Seen,
        "the sixth tick"
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(sighting_of(&app, 0, shade), Sighting::Glimpsed);
}

#[test]
fn watch_naming_ground_leaves_cloaked_flier_glimpsed() {
    let mut app = concealment_app(rivals());
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (shade, _) = utils::create_owned(&mut app, "shade", 8, 5, 1);
    let (phantom, _) = utils::create_owned(&mut app, "phantom", 8, 6, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    utils::use_skill(
        &mut app,
        "ground_sweep",
        SkillCasterRef::Entity(mage_id),
        Some(SkillTarget::Position(utils::pos(8, 5))),
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(sighting_of(&app, 0, shade), Sighting::Seen);
    assert_eq!(sighting_of(&app, 0, phantom), Sighting::Glimpsed);
}

#[test]
fn free_seat_and_unknown_player_see_nothing() {
    let mut app = concealment_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::free(1),
    ]);
    // An exposed unit in the open: any seated side would see it.
    let (scout, _) = utils::create_owned(&mut app, "scout", 5, 5, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 0, scout), Sighting::Seen);

    assert_eq!(
        sighting_of(&app, 1, scout),
        Sighting::Unseen,
        "nobody sits there"
    );
    assert_eq!(
        sighting_of(&app, 7, scout),
        Sighting::Unseen,
        "no such seat"
    );
}

#[test]
fn concealed_unit_auto_engages_as_any_other() {
    let mut app = concealment_app(rivals());
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "ambush", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());

    // A dummy stands into the ambushing mage's reach: the mage hunts it on
    // its own, and the swing that lands ends the ambush. Acquired within the
    // scan period, then three swings of 5 at a period of 4 in twelve ticks:
    // 20 − 3 × 5 = 5.
    let (dummy, _) = utils::create_owned(&mut app, "dummy", 6, 5, 1);
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::health(&app, dummy), 5);
    assert!(!app.world().entity(mage).contains::<ConcealedComponent>());
}

#[test]
fn position_attack_hits_concealed_unit_on_its_cell() {
    let mut app = concealment_app(rivals());
    let (_, mortar_id) = utils::create_owned(&mut app, "mortar", 5, 5, 1);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 10, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 1, shade), Sighting::Glimpsed);

    // The rival cannot name the shade, but a shell sent to the cell it
    // shimmers on hits whatever stands there: 20 − 10 = 10.
    let due = utils::tick(&app) + utils::APPLY;
    utils::run_ticks_commanding(
        &mut app,
        utils::APPLY + 1,
        1,
        due,
        vec![
            PlayerCommand::SelectById {
                id: mortar_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Attack {
                target: AttackTarget::Position(utils::pos(5, 10)),
                flush: true,
            },
        ],
    );
    // The shell flies five cells at a cell a tick and lands on the fifth:
    // 20 − 10 = 10.
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::health(&app, shade),
        10,
        "the shell found what stood there"
    );
}

#[test]
fn splash_hits_concealed_unit_beside_what_it_was_aimed_at() {
    let mut app = concealment_app(rivals());
    let (_, mortar_id) = utils::create_owned(&mut app, "mortar", 5, 5, 1);
    let (dummy, dummy_id) = utils::create_owned(&mut app, "dummy", 5, 10, 0);
    let (shade, _) = utils::create_owned(&mut app, "shade", 5, 11, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(sighting_of(&app, 1, shade), Sighting::Glimpsed);

    // The shell is aimed at the dummy the rival can see; the blast band one
    // cell out catches the shade it cannot: 20 − 5 = 15.
    let due = utils::tick(&app) + utils::APPLY;
    utils::run_ticks_commanding(
        &mut app,
        utils::APPLY + 1,
        1,
        due,
        vec![
            PlayerCommand::SelectById {
                id: mortar_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Attack {
                target: AttackTarget::Entity(dummy_id),
                flush: true,
            },
        ],
    );
    // The shell lands on the fifth tick: 20 − 10 direct on the dummy, and
    // the band one cell out deals half of it to the shade, 20 − 5 = 15.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health(&app, dummy), 10, "the direct hit");
    assert_eq!(utils::health(&app, shade), 15, "and the blast one cell out");
}

#[test]
fn sight_reaching_one_corner_of_body_makes_out_whole_of_it() {
    let mut app = concealment_app(rivals());
    // Sight is stamped as a circle. From (11, 11) with range 3 it reaches
    // (9, 9), 2.83 cells away, and not (7, 7), 5.66 away. The hall spans
    // (7, 7) to (9, 9), so the one lit cell is its far corner — deliberately
    // not the corner it anchors at, which is the cell the rule used to read.
    utils::create_owned(&mut app, "dummy", 11, 11, 0);
    let (hall, _) = utils::create_owned(&mut app, "hall", 7, 7, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(
        sighting_of(&app, 0, hall),
        Sighting::Seen,
        "any one lit cell of a footprint settles it"
    );
}

#[test]
fn sight_reaching_no_cell_of_body_makes_out_none_of_it() {
    let mut app = concealment_app(rivals());
    // Two cells further out than the test above: (5, 5) to (7, 7), whose
    // nearest corner is 5.66 away, past the dummy's three cells of sight.
    utils::create_owned(&mut app, "dummy", 11, 11, 0);
    let (hall, _) = utils::create_owned(&mut app, "hall", 5, 5, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(sighting_of(&app, 0, hall), Sighting::Unseen);
}

#[test]
fn detector_reaching_one_corner_of_concealed_body_makes_out_whole_of_it() {
    let mut app = concealment_app(rivals());
    // The eye sees eight cells and detects six, both as circles from (12, 12).
    // The crypt spans (6, 6) to (8, 8): its far corner (8, 8) is 5.66 away and
    // detected, while the corner it anchors at is 8.49 away and is not.
    utils::create_owned(&mut app, "ground_eye", 12, 12, 0);
    let (crypt, _) = utils::create_owned(&mut app, "crypt", 6, 6, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(
        sighting_of(&app, 0, crypt),
        Sighting::Seen,
        "any one detected cell of a footprint settles it"
    );
}

#[test]
fn concealed_body_lit_but_undetected_anywhere_is_glimpsed() {
    let mut app = concealment_app(rivals());
    // Sight enough to light the far corner at 2.83, and no detector at all.
    utils::create_owned(&mut app, "dummy", 11, 11, 0);
    let (crypt, _) = utils::create_owned(&mut app, "crypt", 7, 7, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(sighting_of(&app, 0, crypt), Sighting::Glimpsed);
}

#[test]
fn glimpse_carries_whole_footprint_not_one_cell() {
    let mut app = concealment_app(rivals());
    // Lit at its far corner from (11, 11), undetected, so the crypt is a
    // glimpse: three by three, anchored at (7, 7).
    utils::create_owned(&mut app, "dummy", 11, 11, 0);
    utils::create_owned(&mut app, "crypt", 7, 7, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    let view = game_view(
        app.world(),
        0,
        "test",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    assert!(view.enemy_entities.is_empty(), "a glimpse names no entity");
    assert_eq!(
        view.glimpses,
        vec![Glimpse {
            x: 7,
            y: 7,
            width: 3,
            height: 3,
        }],
        "the ground it stands on is the whole footprint"
    );
}

//
// ─── Fields ──────────────────────────────────────────────────────────────────
//

#[test]
fn veil_conceals_allied_units_inside_and_not_its_source() {
    let mut app = concealment_app(rivals());
    let (arbiter, arbiter_id) = utils::create_owned(&mut app, "arbiter", 5, 5, 0);
    let (zealot, _) = utils::create_owned(&mut app, "zealot", 6, 6, 0);
    utils::create_owned(&mut app, "scout", 9, 9, 1);
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(app.world().entity(zealot).contains::<ConcealedComponent>());
    assert!(!app.world().entity(arbiter).contains::<ConcealedComponent>());
    assert_eq!(sighting_of(&app, 1, zealot), Sighting::Glimpsed);
    assert_eq!(sighting_of(&app, 1, arbiter), Sighting::Seen);

    // The veil moves with the arbiter, and the zealot is exposed once it is gone.
    utils::select(&mut app, arbiter_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(25, 25),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 60);
    assert!(!app.world().entity(zealot).contains::<ConcealedComponent>());
    assert_eq!(sighting_of(&app, 1, zealot), Sighting::Seen);
}

#[test]
fn veil_hides_wide_body_only_once_it_covers_every_cell() {
    // The veil reaches three cells from the arbiter at (5, 5), as a circle: of
    // a two-by-two body anchored at (7, 3) it covers (7, 3) at 2.83 and (7, 4)
    // at 2.24, and not (8, 3) at 3.61 or (8, 4) at 3.16. The rival scout at
    // (9, 9) sees six cells, which lights (8, 4) at 5.10 — one cell of either
    // body, and one is enough to make it out.
    for (type_name, concealed, sighting) in [
        ("rabble", true, Sighting::Glimpsed),
        ("phalanx", false, Sighting::Seen),
    ] {
        let mut app = concealment_app(rivals());
        utils::create_owned(&mut app, "arbiter", 5, 5, 0);
        let (body, _) = utils::create_owned(&mut app, type_name, 7, 3, 0);
        utils::create_owned(&mut app, "scout", 9, 9, 1);
        utils::run_ticks(&mut app, utils::APPLY);

        // The rabble answers to any covered cell and hides; the phalanx asks
        // for every cell and stands in the open with two of them bare.
        assert_eq!(
            app.world().entity(body).contains::<ConcealedComponent>(),
            concealed,
            "{type_name}"
        );
        assert_eq!(sighting_of(&app, 1, body), sighting, "{type_name}");
    }
}

#[test]
fn landing_under_veil_keeps_marker_through_its_tick() {
    let mut app = concealment_app(rivals());
    utils::create_owned(&mut app, "arbiter", 5, 5, 0);
    let (zealot, _) = utils::create_owned(&mut app, "zealot", 6, 6, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().entity(zealot).contains::<ConcealedComponent>());

    // A two-tick change into the sentinel, which answers to the veil as the
    // zealot does. The marker is refitted before the orders run, so the tick
    // the change lands in is the one a landing that dropped the marker would
    // show bare, until the refit after.
    utils::order_morph(&mut app, zealot, "sentinel");
    let mut landed = false;
    for _ in 0..3 {
        utils::run_ticks(&mut app, 1);
        if entity_def::of(app.world(), zealot).name == "sentinel" {
            landed = true;
            break;
        }
    }
    assert!(landed, "a two-tick change lands within three ticks");
    assert!(
        app.world().entity(zealot).contains::<ConcealedComponent>(),
        "the landing keeps the marker the veil earns"
    );
}

//
// ─── Buffs ───────────────────────────────────────────────────────────────────
//

#[test]
fn upkeep_buff_drains_energy_each_period_and_ends_when_pool_is_dry() {
    let mut app = concealment_app(rivals());
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    utils::run_ticks(&mut app, utils::APPLY);

    utils::use_skill(&mut app, "cloak", SkillCasterRef::Entity(mage_id), None);
    // The cast lands on the third tick and costs 20 of 100; the marker is
    // refitted the tick after, and the first upkeep of 10 falls due a full
    // period — two ticks — after the cast: four ticks on, 80 energy and
    // concealed, nothing paid yet.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::energy(&app, mage), utils::fixed("80"));
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());

    // Fourteen ticks on, seven payments have fallen due (at 2, 4, … 14 ticks
    // after the cast): 80 − 7 × 10 = 10.
    utils::run_ticks(&mut app, 14);
    assert_eq!(utils::energy(&app, mage), utils::fixed("10"));
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());

    // The eighth payment empties the pool; the ninth cannot be made, so the
    // buff ends the tick it falls due and the marker follows at the next
    // refit: 10 − 10 = 0, and unconcealed within four more ticks.
    utils::run_ticks(&mut app, 4);
    assert!(!app.world().entity(mage).contains::<ConcealedComponent>());
    assert_eq!(utils::energy(&app, mage), FixedU64::ZERO);
}

#[test]
fn resource_upkeep_pays_from_stockpile_and_ends_when_it_cannot() {
    let mut app = concealment_app(rivals());
    utils::record_announcements(&mut app);
    let (lurker, lurker_id) = utils::create_owned(&mut app, "lurker", 5, 5, 0);
    utils::grant_gold(&mut app, 12);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "hire", SkillCasterRef::Entity(lurker_id), None);
    // The cast lands on the third tick, free; the marker follows the tick
    // after, and nothing is paid until the first period runs out.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::gold(app.world()), 12);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());

    // Five ticks after the cast the first 5 gold are paid, five later the
    // second: 12 − 5 − 5 = 2.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::gold(app.world()), 7);
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::gold(app.world()), 2);

    // The third payment cannot be met: the buff ends the tick it falls due,
    // the 2 gold stay, and the marker goes at the next refit.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::gold(app.world()), 2);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());
    utils::run_ticks(&mut app, 1);
    assert!(!app.world().entity(lurker).contains::<ConcealedComponent>());
    assert_eq!(utils::gold(app.world()), 2);
    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::ResourcesSpent {
                    cause: SpendCause::Upkeep { bearer, .. },
                    ..
                } if *bearer == lurker_id
            )),
        "the upkeep is announced as the bearer's"
    );
}

#[test]
fn unowned_bearer_loses_upkeep_buff_at_first_due_tick() {
    let mut app = concealment_app(rivals());
    let (lurker, _) =
        utils::create_entity(app.world_mut(), "lurker", utils::pos(5, 5), None).unwrap();
    let hired = app
        .world()
        .resource::<ContentRegistry>()
        .entity_buff("hired")
        .expect("the fixture registers the hired buff");
    stats::apply_entity_buff(app.world_mut(), lurker, hired);
    utils::run_ticks(&mut app, 1);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());

    // Nobody pays for the unowned: the buff ends the tick its first payment
    // falls due, five ticks after the one it was applied in, and the marker
    // follows at the next refit.
    utils::run_ticks(&mut app, 5);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());
    utils::run_ticks(&mut app, 1);
    assert!(!app.world().entity(lurker).contains::<ConcealedComponent>());
}

#[test]
fn health_upkeep_ends_buff_and_never_kills() {
    let mut app = concealment_app(rivals());
    let (lurker, _) = utils::create_owned(&mut app, "lurker", 5, 5, 0);
    let bleeding = app
        .world()
        .resource::<ContentRegistry>()
        .entity_buff("bleeding")
        .expect("the fixture registers the bleeding buff");
    stats::apply_entity_buff(app.world_mut(), lurker, bleeding);

    // Period 1, seated at 2: the first 8 health fall due on the second tick,
    // the second 8 on the third. 20 − 8 = 12, then 12 − 8 = 4.
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::health(&app, lurker), 12);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, lurker), 4);

    // The third would not leave the lurker alive, so it is refused: the buff
    // ends the tick it falls due, the marker goes at the refit after, and the
    // 4 health stay.
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, lurker), 4);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());
    utils::run_ticks(&mut app, 1);
    assert!(!app.world().entity(lurker).contains::<ConcealedComponent>());
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::health(&app, lurker), 4, "an upkeep never kills");
    assert!(!app.world().entity(lurker).contains::<DyingComponent>());
}

#[test]
fn decloak_removes_upkeep_buff() {
    let mut app = concealment_app(rivals());
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "cloak", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());

    utils::use_skill(&mut app, "decloak", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!app.world().entity(mage).contains::<ConcealedComponent>());
}

#[test]
fn ambush_ends_when_bearer_attacks() {
    let mut app = concealment_app(rivals());
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    let (dummy, dummy_id) = utils::create_owned(&mut app, "dummy", 6, 5, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "ambush", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());

    utils::select(&mut app, mage_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(dummy_id),
            flush: true,
        },
    );
    // The first swing lands within a handful of ticks and cuts the ambush
    // short; over ten ticks three swings of 5 land at a period of 4:
    // 20 − 3 × 5 = 5.
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::health(&app, dummy), 5, "the swings landed");
    assert!(!app.world().entity(mage).contains::<ConcealedComponent>());
}

#[test]
fn ambush_ends_when_bearer_is_hit() {
    let mut app = concealment_app(rivals());
    // A weaponless bearer, so nothing but the hit can end the ambush; the
    // enemy sniper needs a detector to hit what hides.
    let (lurker, lurker_id) = utils::create_owned(&mut app, "lurker", 5, 5, 0);
    utils::create_owned(&mut app, "sniper", 5, 9, 1);
    utils::create_owned(&mut app, "tower", 7, 7, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "ambush", SkillCasterRef::Entity(lurker_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());

    // Detected and shot: the sniper's first hit of 10 lands, and the marker
    // goes at the refit that follows it.
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, lurker), 10, "one hit of 10 landed");
    assert!(!app.world().entity(lurker).contains::<ConcealedComponent>());
}

#[test]
fn damaging_cast_ends_ambush_of_its_target() {
    let mut app = concealment_app(rivals());
    let (lurker, lurker_id) = utils::create_owned(&mut app, "lurker", 5, 5, 0);
    // The rival mage needs a detector to aim its bolt at what hides.
    let (_, mage_id) = utils::create_owned(&mut app, "mage", 5, 9, 1);
    utils::create_owned(&mut app, "tower", 7, 7, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "ambush", SkillCasterRef::Entity(lurker_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());

    // A frame is committed an input delay ahead of the tick it runs at; the
    // bolt lands 10 of the lurker's 20, and the hit ends the ambush.
    let due = utils::tick(&app) + utils::APPLY;
    let bolt = utils::skill_id(&app, "bolt");
    utils::run_ticks_commanding(
        &mut app,
        utils::APPLY + 3,
        1,
        due,
        vec![PlayerCommand::UseSkill {
            skill: bolt,
            caster: SkillCasterRef::Entity(mage_id),
            target: Some(SkillTarget::Entity(lurker_id)),
        }],
    );
    assert_eq!(utils::health(&app, lurker), 10);
    assert!(!app.world().entity(lurker).contains::<ConcealedComponent>());
}

#[test]
fn turret_gun_ends_ambush_of_its_bearer() {
    let mut app = concealment_app(rivals());
    // The gunner's turret hunts on its own; its shot is the bearer's attack.
    let (gunner, gunner_id) = utils::create_owned(&mut app, "gunner", 5, 5, 0);
    let (dummy, _) = utils::create_owned(&mut app, "dummy", 6, 5, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "ambush", SkillCasterRef::Entity(gunner_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(gunner).contains::<ConcealedComponent>());

    // The gun slews three degrees a tick and needs twenty to bear on the
    // dummy; its first shot of 5 lands on the twenty-first and ends the
    // ambush, its second not before the twenty-fifth: 20 − 5 = 15.
    utils::run_ticks(&mut app, 24);
    assert_eq!(utils::health(&app, dummy), 15);
    assert!(!app.world().entity(gunner).contains::<ConcealedComponent>());
}

#[test]
fn projectile_turret_ends_ambush_when_its_shot_leaves() {
    let mut app = concealment_app(rivals());
    // The lobber's turret sends a shell to a cell: the ambush ends when the
    // shot leaves, not when it lands.
    let (lobber, lobber_id) = utils::create_owned(&mut app, "lobber", 5, 5, 0);
    let (dummy, _) = utils::create_owned(&mut app, "dummy", 5, 9, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "ambush", SkillCasterRef::Entity(lobber_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(lobber).contains::<ConcealedComponent>());

    // The gun bears at once and the shot leaves on the second tick: the
    // ambush ends there, with the shell still in the air. It lands three
    // ticks later: 20 − 5 = 15.
    utils::run_ticks(&mut app, 2);
    assert!(!app.world().entity(lobber).contains::<ConcealedComponent>());
    assert_eq!(
        utils::health(&app, dummy),
        20,
        "the shell is still in flight"
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::health(&app, dummy), 15, "and then it lands");
}

#[test]
fn swing_canceled_before_its_point_keeps_ambush() {
    let mut app = concealment_app(rivals());
    // The spearman's swing reaches its point ten ticks in. The dummy stands
    // three cells off: inside the spear's reach of three, outside the one
    // cell the spearman notices on its own, so nothing but the order swings.
    let (spearman, spearman_id) = utils::create_owned(&mut app, "spearman", 5, 5, 0);
    let (dummy, dummy_id) = utils::create_owned(&mut app, "dummy", 5, 8, 1);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(
        &mut app,
        "ambush",
        SkillCasterRef::Entity(spearman_id),
        None,
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(
        app.world()
            .entity(spearman)
            .contains::<ConcealedComponent>()
    );

    utils::select(&mut app, spearman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(dummy_id),
            flush: true,
        },
    );
    // The order lands three ticks on and the swing runs four of its ten.
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_eq!(utils::health(&app, dummy), 20, "the point is not reached");
    assert!(!utils::order_queue_is_empty(app.world_mut(), spearman));

    // Called off mid-swing, three ticks on again, eight ticks into the ten:
    // the swing is abandoned and no shot leaves. An attack cuts the ambush at
    // its point alone, so the ambush stands, and nobody swings at a dummy the
    // spearman does not notice.
    utils::push_command(&mut app, PlayerCommand::Stop);
    utils::run_ticks(&mut app, utils::APPLY + 20);
    assert!(utils::order_queue_is_empty(app.world_mut(), spearman));
    assert_eq!(utils::health(&app, dummy), 20, "no shot left");
    assert!(
        app.world()
            .entity(spearman)
            .contains::<ConcealedComponent>()
    );
}

#[test]
fn cast_ends_buff_interrupted_by_casting() {
    let mut app = concealment_app(rivals());
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "focus", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());

    // The daze is a cast: it ends the focus as it lands, and its own buff —
    // applied after the cut — stays.
    utils::use_skill(&mut app, "daze", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!app.world().entity(mage).contains::<ConcealedComponent>());
    assert_eq!(
        entity_def::operation(app.world(), mage),
        Operation::Disabled(Outage::Buff)
    );
}

#[test]
fn timed_concealment_ends_with_its_ticks() {
    let mut app = concealment_app(rivals());
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "ambush", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());

    // Fifty ticks of ambush after the tick the cast landed in: concealed on
    // the fiftieth, and the marker goes at the refit after the buff drops.
    utils::run_ticks(&mut app, 49);
    assert!(app.world().entity(mage).contains::<ConcealedComponent>());
    utils::run_ticks(&mut app, 1);
    assert!(!app.world().entity(mage).contains::<ConcealedComponent>());
}

#[test]
fn disabling_buff_stops_operation() {
    let mut app = concealment_app(rivals());
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 5, 0);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::use_skill(&mut app, "daze", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        entity_def::operation(app.world(), mage),
        Operation::Disabled(Outage::Buff)
    );

    // A dazed mage starts no walk.
    utils::select(&mut app, mage_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(10, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::position_of(app.world(), mage), utils::pos(5, 5));

    // Twenty ticks of daze after the tick the cast landed in — the five run
    // so far and fourteen more make the twentieth — then it operates again.
    utils::run_ticks(&mut app, 14);
    assert_eq!(
        entity_def::operation(app.world(), mage),
        Operation::Disabled(Outage::Buff)
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::operation(app.world(), mage),
        Operation::Operating
    );
}

#[test]
fn stacked_upkeep_pays_for_every_stack() {
    let mut app = concealment_app(rivals());
    let (lurker, _) = utils::create_owned(&mut app, "lurker", 5, 5, 0);
    utils::grant_gold(&mut app, 30);
    let retained = app
        .world()
        .resource::<ContentRegistry>()
        .entity_buff("retained")
        .expect("the fixture registers the retained buff");
    for _ in 0..3 {
        stats::apply_entity_buff(app.world_mut(), lurker, retained);
    }

    // Period 5, seated at 6 because the tick of application ages it once.
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::gold(app.world()),
        30,
        "nothing is due before the period runs out"
    );

    utils::run_ticks(&mut app, 1);
    // 30 granted − 3 stacks × 5 gold: the modifiers apply per stack, so the
    // upkeep is owed per stack.
    assert_eq!(utils::gold(app.world()), 15);
}

#[test]
fn stack_added_mid_period_keeps_first_payment_due() {
    let mut app = concealment_app(rivals());
    let (lurker, _) = utils::create_owned(&mut app, "lurker", 5, 5, 0);
    utils::grant_gold(&mut app, 30);
    let retained = app
        .world()
        .resource::<ContentRegistry>()
        .entity_buff("retained")
        .expect("the fixture registers the retained buff");
    stats::apply_entity_buff(app.world_mut(), lurker, retained);

    // Period 5, seated at 6. A second stack four ticks in joins the first at
    // its countdown rather than starting one of its own.
    utils::run_ticks(&mut app, 4);
    stats::apply_entity_buff(app.world_mut(), lurker, retained);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::gold(app.world()),
        30,
        "nothing is due on the fifth tick"
    );

    // Both stacks fall due on the sixth: 30 − 2 × 5 = 20.
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::gold(app.world()), 20);
}

//
// ─── Orders lapsing with sight ───────────────────────────────────────────────
//

#[test]
fn attack_order_lapses_when_target_cloaks() {
    let mut app = concealment_app(rivals());
    // The rival's sniper engages the mage on its own initiative; the mage is
    // the local player's, so the cloak is the local player's to order.
    let (sniper, _) = utils::create_owned(&mut app, "sniper", 5, 5, 1);
    let (mage, mage_id) = utils::create_owned(&mut app, "mage", 5, 8, 0);
    // The sniper acquires within its scan period and lands 10 every 2 ticks:
    // three hits in twelve ticks, 100 − 3 × 10 = 70.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::health(&app, mage), 70, "the attack is under way");
    assert!(!utils::order_queue_is_empty(app.world_mut(), sniper));

    utils::use_skill(&mut app, "cloak", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(utils::order_queue_is_empty(app.world_mut(), sniper));
    // Nothing lands on what the sniper cannot make out any more.
    let after = utils::health(&app, mage);
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::health(&app, mage), after);
}

#[test]
fn attack_order_lapses_when_target_walks_out_of_sight() {
    let mut app = concealment_app(rivals());
    // The rival's marksman shoots to ten but sees to four, and is ordered to
    // attack the local runner three cells off; the runner is ordered away at
    // the same time, to seven cells — still in range, out of sight — so the
    // attack lands while it can and lapses when the runner leaves the light.
    let (marksman, marksman_id) = utils::create_owned(&mut app, "marksman", 5, 5, 1);
    let (runner, runner_id) = utils::create_owned(&mut app, "runner", 5, 8, 0);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    utils::select(&mut app, runner_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(5, 12),
            flush: true,
        },
    );
    let due = utils::tick(&app) + utils::APPLY;
    utils::run_ticks_commanding(
        &mut app,
        40,
        1,
        due,
        vec![
            PlayerCommand::SelectById {
                id: marksman_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Attack {
                target: AttackTarget::Entity(runner_id),
                flush: true,
            },
        ],
    );
    // Two hits of 10 land before the runner leaves the light, then the order
    // lapses: 500 − 2 × 10 = 480.
    assert_eq!(utils::health(&app, runner), 480);
    assert!(utils::order_queue_is_empty(app.world_mut(), marksman));
    let after = utils::health(&app, runner);
    utils::run_ticks(&mut app, 10);
    assert_eq!(
        utils::health(&app, runner),
        after,
        "nothing lands on what is out of sight"
    );
}

#[test]
fn leashed_attack_under_seat_detecting_everywhere_lapses_when_detector_dies() {
    let mut app = concealment_app(seat_detecting_everywhere());
    // The seat names what no detector shows it, but its sniper picks this
    // fight for itself, by the ordinary senses: the tower's detection.
    let (sniper, _) = utils::create_owned(&mut app, "sniper", 5, 5, 1);
    let (tower, _) = utils::create_owned(&mut app, "tower", 7, 7, 1);
    let (wight, _) = utils::create_owned(&mut app, "wight", 5, 8, 0);
    // Acquired within the scan period and hit for 10 every 2 ticks: three
    // hits in twelve ticks, 500 − 3 × 10 = 470.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::health(&app, wight), 470, "the fight is under way");
    assert!(!utils::order_queue_is_empty(app.world_mut(), sniper));

    // The detector goes, and with it the ordinary sight of the wight: the
    // leashed attack lapses though the seat itself still names the wight.
    spawn::destroy_entity(app.world_mut(), tower);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(utils::order_queue_is_empty(app.world_mut(), sniper));
    assert_eq!(
        sighting_of(&app, 1, wight),
        Sighting::Seen,
        "the seat still names it"
    );
    utils::run_ticks(&mut app, 10);
    assert_eq!(
        utils::health(&app, wight),
        470,
        "nothing lands on what the ordinary senses lost"
    );
}

#[test]
fn leashed_attack_under_seat_detecting_everywhere_continues_while_detector_stands() {
    let mut app = concealment_app(seat_detecting_everywhere());
    let (sniper, _) = utils::create_owned(&mut app, "sniper", 5, 5, 1);
    utils::create_owned(&mut app, "tower", 7, 7, 1);
    let (wight, _) = utils::create_owned(&mut app, "wight", 5, 8, 0);
    // Three hits in twelve ticks, as above: 500 − 3 × 10 = 470.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::health(&app, wight), 470);

    // The tower stands, so the detector the fight was picked under still
    // covers the wight: five more hits in the next ten ticks, 470 − 5 × 10 =
    // 420, and the leashed attack holds.
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::health(&app, wight), 420);
    assert!(!utils::order_queue_is_empty(app.world_mut(), sniper));
}

#[test]
fn follow_order_lapses_when_target_cloaks() {
    let mut app = concealment_app(rivals());
    // The local scout tails the rival's lurker; the cloak is the rival's to
    // order, so it travels in that player's own frame.
    let (scout, scout_id) = utils::create_owned(&mut app, "scout", 5, 5, 0);
    let (_, lurker_id) = utils::create_owned(&mut app, "lurker", 5, 8, 1);
    utils::select(&mut app, scout_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Follow {
            target: lurker_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(!utils::order_queue_is_empty(app.world_mut(), scout));
    assert!(app.world().entity(scout).contains::<FollowComponent>());

    let cloak = utils::skill_id(&app, "cloak");
    // A frame is committed an input delay ahead of the tick it runs at.
    let due = utils::tick(&app) + utils::APPLY;
    utils::run_ticks_commanding(
        &mut app,
        12,
        1,
        due,
        vec![PlayerCommand::UseSkill {
            skill: cloak,
            caster: SkillCasterRef::Entity(lurker_id),
            target: None,
        }],
    );
    // The order lapses with the sight, and its driver goes with the order.
    assert!(utils::order_queue_is_empty(app.world_mut(), scout));
    assert!(!app.world().entity(scout).contains::<FollowComponent>());
}

#[test]
fn follow_order_lapses_when_target_walks_out_of_sight() {
    let mut app = concealment_app(rivals());
    // The scout sees six and walks at half a cell a tick; the rival's
    // sprinter runs at one and a half, so a follow falls behind and the
    // order lapses when the gap passes the scout's sight.
    let (scout, scout_id) = utils::create_owned(&mut app, "scout", 5, 5, 0);
    let (_, sprinter_id) = utils::create_owned(&mut app, "sprinter", 5, 8, 1);
    utils::select(&mut app, scout_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Follow {
            target: sprinter_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(!utils::order_queue_is_empty(app.world_mut(), scout));

    // The sprinter is sent to the far side of the map, out of the scout's
    // sight long before it arrives.
    let due = utils::tick(&app) + utils::APPLY;
    utils::run_ticks_commanding(
        &mut app,
        30,
        1,
        due,
        vec![
            PlayerCommand::SelectById {
                id: sprinter_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Move {
                target: utils::pos(5, 30),
                flush: true,
            },
        ],
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), scout),
        "the follow lapsed with the sight"
    );
}

#[test]
fn guard_order_lapses_when_ward_cloaks() {
    let mut app = concealment_app(rivals());
    // A guard on a rival's unit is refused outright, so the ward is nobody's:
    // a stray lurker the scout may escort, and cannot make out once it hides.
    let (scout, scout_id) = utils::create_owned(&mut app, "scout", 5, 5, 0);
    let (lurker, lurker_id) =
        utils::create_entity(app.world_mut(), "lurker", utils::pos(5, 8), None)
            .expect("the lurker fits at (5, 8)");
    utils::select(&mut app, scout_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Guard {
            target: lurker_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(!utils::order_queue_is_empty(app.world_mut(), scout));
    assert!(app.world().entity(scout).contains::<GuardComponent>());

    // Nobody pays an upkeep for the unowned, so what hides the lurker is the
    // timed ambush, which a bearer with no weapon never cuts short.
    let ambushing = app
        .world()
        .resource::<ContentRegistry>()
        .entity_buff("ambushing")
        .expect("the fixture registers the ambushing buff");
    stats::apply_entity_buff(app.world_mut(), lurker, ambushing);
    utils::run_ticks(&mut app, 12);
    assert!(app.world().entity(lurker).contains::<ConcealedComponent>());
    // The order lapses with the sight, and its driver goes with the order.
    assert!(utils::order_queue_is_empty(app.world_mut(), scout));
    assert!(!app.world().entity(scout).contains::<GuardComponent>());
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Two rival humans.
fn rivals() -> Vec<PlayerSlot> {
    utils::human_slots(2)
}

/// A local human against a scripted seat that sees the whole map and names
/// every concealed body on it.
fn seat_detecting_everywhere() -> Vec<PlayerSlot> {
    vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Omniscient,
                detection: AiDetection::Everywhere,
            },
            None,
            None,
        ),
    ]
}

/// What `player`'s side makes of `entity` now.
fn sighting_of(app: &App, player: PlayerId, entity: Entity) -> Sighting {
    visibility::sighting(app.world(), player, entity, Senses::SeatDeclared)
}

/// A free, instant skill an entity aims at `target` with the given effect.
fn aimed(target: EntityCastTarget, effect: EntityCastEffect) -> SkillDef {
    SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            target,
            reach: Reach::Wherever,
            casting: Casting::Instant,
            effect,
        },
        requires: Vec::new(),
    }
}

/// An instant skill an entity casts on itself with the given effect, at the
/// given costs.
fn self_cast(effect: EntityCastEffect, costs: Vec<Cost>) -> SkillDef {
    SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs,
            target: EntityCastTarget::Caster,
            reach: Reach::Wherever,
            casting: Casting::Instant,
            effect,
        },
        requires: Vec::new(),
    }
}

/// A one-cell walker with `health` and `sight`.
fn walker(name: &str, occupation: impl Into<LayerMask>, health: u32, sight: u32) -> EntityTypeDef {
    utils::walker(name, occupation)
        .with_health(health)
        .with_dying(1, [])
        .with_sight_range(sight)
}

/// App with concealment content and the session started: a `scout` that sees
/// six, a `sniper` that shoots and sees eight, a `marksman` that shoots ten and
/// sees four, a `runner` too tough to die, a permanently concealed `shade` and
/// a `wight` that is the shade too tough to die and rooted where it stands, a concealed `phantom` in the
/// air, a `dummy`, a 3×3 `hall` and a concealed 3×3 `crypt`; a `tower` and a
/// `ground_eye` detecting to six on every layer and on the ground alone, the
/// tower blind and halted under an enemy veil; an `arbiter` veiling allied
/// units around it, a `zealot` that answers to the veil and changes into a
/// `sentinel` that answers to it too, and a 2×2 `phalanx` and `rabble` that
/// answer to it over every cell and over any; a `mage` with the `cloak`,
/// `decloak`, `ambush`, `daze`, `focus`, `sweep`, `ground_sweep` and `bolt`
/// skills; a weaponless `lurker` with `cloak`, `ambush` and `hire`; a
/// `spearman` whose swing takes ten ticks to land, a `gunner` whose turret
/// shoots the ground and a `lobber` whose turret lobs a shell at a cell, all
/// three with `ambush`; a `mortar` that sends a bursting shell to a cell; and
/// a `sprinter` that outruns anything following it.
fn concealment_app(slots: Vec<PlayerSlot>) -> App {
    let mut app = utils::make_app(slots);
    // The air layer the phantom and the arbiter live on, over the same ground.
    {
        let mut grid = NavGrid::new(32, 32);
        grid.add_layer(utils::GROUND);
        grid.add_layer(utils::AIR);
        map::install_map(
            app.world_mut(),
            Map::new(
                "test",
                Projection::Isometric,
                MovementModel::Cell,
                grid,
                vec![],
                &[
                    MoverShape::point(utils::GROUND),
                    MoverShape::point(utils::AIR),
                ],
            ),
        );
    }
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(utils::AIR_LAYER), utils::AIR);
        let air = utils::AIR;

        let true_sight = registry.register_field(
            "true_sight",
            FieldDef::new(
                FieldLayer::Anywhere,
                FieldDecay::Instant,
                FieldVision::Dark,
                Detection::Reveals(LayerMask::EMPTY | utils::GROUND | air),
            ),
        );
        let ground_sight = registry.register_field(
            "ground_sight",
            FieldDef::new(
                FieldLayer::Anywhere,
                FieldDecay::Instant,
                FieldVision::Dark,
                Detection::Reveals(utils::GROUND.into()),
            ),
        );
        let veil = registry.register_field(
            "veil",
            FieldDef::new(
                FieldLayer::Anywhere,
                FieldDecay::Instant,
                FieldVision::Dark,
                Detection::Blind,
            ),
        );

        let cloaked = registry.register_entity_buff(
            "cloaked",
            EntityBuffDef {
                effects: vec![EntityEffect::Conceal],
                lasting: Lasting::Upkeep {
                    costs: vec![Cost::Energy(FixedU64::from_num(10))],
                    period: 2,
                },
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        let ambushing = registry.register_entity_buff(
            "ambushing",
            EntityBuffDef {
                effects: vec![EntityEffect::Conceal],
                lasting: Lasting::For(50),
                stack_rule: StackRule::Refresh,
                interrupted_by: vec![Interruption::Attack, Interruption::Hit],
            },
        );
        let dazed = registry.register_entity_buff(
            "dazed",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::For(20),
                stack_rule: StackRule::Refresh,
                interrupted_by: Vec::new(),
            },
        );
        let focused = registry.register_entity_buff(
            "focused",
            EntityBuffDef {
                effects: vec![EntityEffect::Conceal],
                lasting: Lasting::For(50),
                stack_rule: StackRule::Refresh,
                interrupted_by: vec![Interruption::Cast],
            },
        );
        registry.register_resource("gold");
        let hired = registry.register_entity_buff(
            "hired",
            EntityBuffDef {
                effects: vec![EntityEffect::Conceal],
                lasting: Lasting::Upkeep {
                    costs: vec![Cost::Resources(price::from([("gold", 5)]))],
                    period: 5,
                },
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        // An upkeep that stacks, to show a bearer pays for every stack it
        // carries rather than for one.
        registry.register_entity_buff(
            "retained",
            EntityBuffDef {
                effects: vec![EntityEffect::Conceal],
                lasting: Lasting::Upkeep {
                    costs: vec![Cost::Resources(price::from([("gold", 5)]))],
                    period: 5,
                },
                stack_rule: StackRule::StackToCap(3),
                interrupted_by: Vec::new(),
            },
        );
        // An upkeep drawn from the bearer's own health, every tick.
        registry.register_entity_buff(
            "bleeding",
            EntityBuffDef {
                effects: vec![EntityEffect::Conceal],
                lasting: Lasting::Upkeep {
                    costs: vec![Cost::Health(FixedU64::from_num(8))],
                    period: 1,
                },
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        let cloak = registry.register_skill(
            "cloak",
            self_cast(
                EntityCastEffect::ApplyBuff(cloaked),
                vec![Cost::Energy(FixedU64::from_num(20))],
            ),
        );
        let decloak = registry.register_skill(
            "decloak",
            self_cast(EntityCastEffect::RemoveBuff(cloaked), Vec::new()),
        );
        let ambush = registry.register_skill(
            "ambush",
            self_cast(EntityCastEffect::ApplyBuff(ambushing), Vec::new()),
        );
        let daze = registry.register_skill(
            "daze",
            self_cast(EntityCastEffect::ApplyBuff(dazed), Vec::new()),
        );
        let focus = registry.register_skill(
            "focus",
            self_cast(EntityCastEffect::ApplyBuff(focused), Vec::new()),
        );
        let hire = registry.register_skill(
            "hire",
            self_cast(EntityCastEffect::ApplyBuff(hired), Vec::new()),
        );
        let sweep = registry.register_skill(
            "sweep",
            aimed(
                EntityCastTarget::Position,
                EntityCastEffect::Watch {
                    radius: 3,
                    duration: 6,
                    detection: Detection::Reveals(LayerMask::EMPTY | utils::GROUND | air),
                },
            ),
        );
        let ground_sweep = registry.register_skill(
            "ground_sweep",
            aimed(
                EntityCastTarget::Position,
                EntityCastEffect::Watch {
                    radius: 3,
                    duration: 6,
                    detection: Detection::Reveals(utils::GROUND.into()),
                },
            ),
        );
        let bolt = registry.register_skill(
            "bolt",
            aimed(
                EntityCastTarget::Standing {
                    side: Affiliation::Enemy,
                    kinds: Kinds::Any,
                },
                EntityCastEffect::Damage(FixedU64::from_num(10)),
            ),
        );
        let gun = registry.register_turret(
            "gun",
            TurretDef::new(
                Weapon::new(utils::GROUND, Delivery::Instant, None, Slain::Remains),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        // A shell sent to a cell, so an attack may name bare ground, and a
        // blast that catches what stands beside what it was aimed at.
        let shell =
            registry.register_projectile("shell", ProjectileDef::new(FixedU64::ONE, Aim::Position));
        let lob = registry.register_turret(
            "lob",
            TurretDef::new(
                Weapon::new(
                    utils::GROUND,
                    Delivery::Projectile(shell),
                    None,
                    Slain::Remains,
                ),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );

        registry.register(walker("scout", utils::GROUND, 20, 6));
        registry.register(walker("sniper", utils::GROUND, 30, 8).with_attack(
            utils::weapon(utils::GROUND),
            10,
            8,
            8,
            2,
            1,
        ));
        registry.register(walker("marksman", utils::GROUND, 30, 4).with_attack(
            utils::weapon(utils::GROUND),
            10,
            10,
            10,
            2,
            1,
        ));
        registry.register(walker("runner", utils::GROUND, 500, 3));
        registry.register(
            walker("shade", utils::GROUND, 20, 3).with_concealment(Concealment::Concealed),
        );
        registry.register(
            EntityTypeDef::new("wight")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(500)
                .with_dying(1, [])
                .with_sight_range(3)
                .with_concealment(Concealment::Concealed),
        );
        registry.register(walker("phantom", air, 20, 3).with_concealment(Concealment::Concealed));
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(1, [])
                .with_sight_range(3),
        );
        // Three by three, so sight and detection can reach one corner of a body
        // and miss the rest: the footprint rules have something to bite on.
        registry.register(
            EntityTypeDef::new("hall")
                .with_location(utils::GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_health(50)
                .with_dying(1, [])
                .with_sight_range(2),
        );
        // The same body, concealed, for the detection half of the rule.
        registry.register(
            EntityTypeDef::new("crypt")
                .with_location(utils::GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_health(50)
                .with_dying(1, [])
                .with_sight_range(2)
                .with_concealment(Concealment::Concealed),
        );
        registry.register(
            EntityTypeDef::new("tower")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50)
                .with_dying(1, [])
                .with_sight_range(2)
                .with_field_sources([FieldSourceDef::new(
                    true_sight,
                    6,
                    FieldGrowth::Instant,
                    Emission::Nothing,
                    Emission::Nothing,
                )])
                .with_field_effects([FieldEffect::new(
                    veil,
                    Affiliation::Enemy,
                    FieldSide::Inside,
                    FieldCoverage::Any,
                    EntityEffect::Disable,
                )]),
        );
        registry.register(
            EntityTypeDef::new("ground_eye")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50)
                .with_dying(1, [])
                .with_sight_range(8)
                .with_field_sources([FieldSourceDef::new(
                    ground_sight,
                    6,
                    FieldGrowth::Instant,
                    Emission::Nothing,
                    Emission::Nothing,
                )]),
        );
        registry.register(walker("arbiter", air, 100, 6).with_field_sources([
            FieldSourceDef::new(
                veil,
                3,
                FieldGrowth::Instant,
                Emission::Nothing,
                Emission::Nothing,
            ),
        ]));
        let veiled = || {
            FieldEffect::new(
                veil,
                Affiliation::Allied,
                FieldSide::Inside,
                FieldCoverage::Any,
                EntityEffect::Conceal,
            )
        };
        registry.register(
            walker("zealot", utils::GROUND, 60, 6)
                .with_field_effects([veiled()])
                .with_morphs([MorphTransition::new(
                    "sentinel",
                    None,
                    Quantity::Constant(2),
                    MorphPlacement::Reserve,
                    MorphCancel::Refundable,
                    MorphInterrupted::Reverts,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                )]),
        );
        registry.register(walker("sentinel", utils::GROUND, 60, 6).with_field_effects([veiled()]));
        // Two wide bodies under the veil, one asking for every cell covered
        // and one for any, so the coverage rule has an edge to stand on.
        registry.register(
            EntityTypeDef::new("phalanx")
                .with_location(utils::GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(50)
                .with_dying(1, [])
                .with_sight_range(2)
                .with_field_effects([FieldEffect::new(
                    veil,
                    Affiliation::Allied,
                    FieldSide::Inside,
                    FieldCoverage::Every,
                    EntityEffect::Conceal,
                )]),
        );
        registry.register(
            EntityTypeDef::new("rabble")
                .with_location(utils::GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(50)
                .with_dying(1, [])
                .with_sight_range(2)
                .with_field_effects([veiled()]),
        );
        registry.register(
            walker("mage", utils::GROUND, 100, 6)
                .with_attack(utils::weapon(utils::GROUND), 5, 1, 5, 4, 1)
                .with_energy(100, FixedU64::ZERO)
                .with_skills([
                    cloak,
                    decloak,
                    ambush,
                    daze,
                    focus,
                    sweep,
                    ground_sweep,
                    bolt,
                ]),
        );
        registry.register(
            walker("lurker", utils::GROUND, 20, 6)
                .with_energy(100, FixedU64::ZERO)
                .with_skills([cloak, ambush, hire]),
        );
        // A swing that takes ten ticks of its twenty to reach its point, and
        // a reach of three cells against a notice of one, so a fight it did
        // not pick for itself is one an order alone starts.
        registry.register(
            walker("spearman", utils::GROUND, 100, 6)
                .with_attack(utils::weapon(utils::GROUND), 5, 3, 1, 20, 10)
                .with_skills([ambush]),
        );
        registry.register(walker("mortar", utils::GROUND, 40, 8).with_attack(
            AttackDef::new(Weapon::new(
                utils::GROUND,
                Delivery::Projectile(shell),
                Some(SplashDef::new(
                    SplashShape::Circular,
                    vec![(1, FixedU64::from_num(0.5))],
                    utils::GROUND,
                    false,
                )),
                Slain::Remains,
            )),
            10,
            8,
            10,
            4,
            1,
        ));
        registry.register(
            walker("sprinter", utils::GROUND, 100, 6)
                .with_stat(EntityStatId::SPEED, FixedU64::from_num(1.5)),
        );
        registry.register(
            walker("lobber", utils::GROUND, 100, 6)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(90))
                .with_stat(EntityStatId::ATTACK_ARC, FixedU64::from_num(360))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(5))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(6))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(8))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(8))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(1))
                .with_turrets([TurretMount::new(lob, CellPos::new(0, 0), CellSize::ONE)])
                .with_skills([ambush]),
        );
        registry.register(
            walker("gunner", utils::GROUND, 100, 6)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(3))
                .with_stat(EntityStatId::ATTACK_ARC, FixedU64::from_num(60))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(5))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(5))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(4))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(1))
                .with_turrets([TurretMount::new(gun, CellPos::new(0, 0), CellSize::ONE)])
                .with_skills([ambush]),
        );
        registry.validate();
    }
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
