//! Fog of war end-to-end: sight reveals cells, exploration is sticky, allied
//! vision is shared, relief hides what rises above a viewer's eyes, objects
//! and fields shadow what lies behind them, combat waits for vision, and the
//! AI view filters fogged enemies only when the brain is fog-limited.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{ai::game_view, map};
use ferrets_content::{
    affiliation::Affiliation,
    detection::Detection,
    domain::DomainDef,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{Emission, FieldDecay, FieldDef, FieldGrowth, FieldSourceDef, FieldWatch},
    layer::LayerLevel,
    location::Solidity,
    pool::Pool,
    quantity::Quantity,
    registry::ContentRegistry,
    sight_block::SightBlock,
    skills::{Casting, EntityCastEffect, Reach, SkillCaster, SkillDef, Spot},
    stats::ModifierOp,
    terrains::Terrains,
};
use ferrets_geometry::{cell_size::CellSize, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_nav_grid::{
    cliff_crossing::CliffCrossing,
    nav_grid::NavGrid,
    slope::{Direction, Slope},
};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::rally::{RallyPointComponent, RallyTarget},
    entity_def,
    map::Map,
    map_data::MapData,
    movement_model::MovementModel,
    order::AttackTarget,
    session::{
        GameSession, ai_detection::AiDetection, ai_vision::AiVision, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    spawn,
    visibility::{CellVisibility, VisibilityGrid},
    watches::Watches,
};

//
// ─── Vision ──────────────────────────────────────────────────────────────────
//

#[test]
fn unit_reveals_cells_within_sight() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    utils::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(visible(&app, 0, 5, 5));
    assert!(visible(&app, 0, 5, 10)); // 5 cells away, within sight 6
    assert!(!visible(&app, 0, 5, 15)); // 10 cells away, beyond sight
}

#[test]
fn structure_sees_from_its_whole_footprint() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    utils::create_entity(app.world_mut(), "keep", utils::pos(10, 10), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    // The keep covers (10..=12, 10..=12); sight 2 reaches two cells past every
    // edge, not two cells past its anchor corner.
    assert!(visible(&app, 0, 8, 10), "two cells left of the footprint");
    assert!(visible(&app, 0, 14, 12), "two cells right of the footprint");
    assert!(visible(&app, 0, 12, 14), "two cells below the footprint");
    assert!(!visible(&app, 0, 15, 12), "three cells right is past sight");
    assert!(!visible(&app, 0, 14, 14), "the corner is farther than two");
}

#[test]
fn exploration_persists_after_unit_moves_away() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let (_, scout) =
        utils::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    utils::select(&mut app, scout);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(25, 25),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 120);

    // The origin is remembered but no longer in sight; the destination is visible.
    assert_eq!(cell_state(&app, 0, 5, 5), CellVisibility::Explored);
    assert!(visible(&app, 0, 25, 25));
}

#[test]
fn allies_share_vision() {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
    ]);
    // The ally (player 1) has the only unit; its sight reaches player 0.
    utils::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(visible(&app, 0, 5, 5));
}

//
// ─── Combat gating ───────────────────────────────────────────────────────────
//

#[test]
fn auto_attack_waits_for_team_vision() {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    // The sniper out-ranges its own sight: the dummy is within weapon/acquire
    // range (8) but outside sight (3), so it stays unseen and unengaged.
    utils::create_entity(app.world_mut(), "sniper", utils::pos(5, 5), Some(0)).unwrap();
    let (dummy, _) =
        utils::create_entity(app.world_mut(), "dummy", utils::pos(11, 5), Some(1)).unwrap();
    utils::run_ticks(&mut app, 20);
    assert!(
        app.world().get_entity(dummy).is_ok(),
        "dummy was attacked while unseen"
    );

    // A scout reveals the dummy to the team; now the sniper engages it.
    utils::create_entity(app.world_mut(), "scout", utils::pos(9, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 30);
    utils::assert_despawned(app.world_mut(), dummy);
}

//
// ─── AI view ─────────────────────────────────────────────────────────────────
//

#[test]
fn ai_view_hides_fogged_enemies_only_when_fog_limited() {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    utils::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
    utils::create_entity(app.world_mut(), "dummy", utils::pos(25, 25), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    let world = app.world();
    let fog_limited = game_view(
        world,
        0,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    let omniscient = game_view(
        world,
        0,
        "human",
        AiVision::Omniscient,
        AiDetection::Detectors,
    );
    assert!(
        fog_limited.enemy_entities.is_empty(),
        "a fog-limited brain must not see the fogged enemy"
    );
    assert_eq!(
        omniscient.enemy_entities.len(),
        1,
        "an omniscient brain sees the enemy regardless of fog"
    );
}

//
// ─── Command gating ──────────────────────────────────────────────────────────
//
// One rule, pinned per command: a target the fog hides cannot be named — the
// executor resolves every named entity through the same sight gate.

#[test]
fn attack_refuses_target_in_weapon_range_but_out_of_sight() {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let world = app.world_mut();
    // The sniper shoots to eight but sees to three: its target stands in
    // weapon range yet out of sight.
    let (sniper, sniper_id) =
        utils::create_entity(world, "sniper", utils::pos(5, 5), Some(0)).unwrap();
    let (mark, mark_id) = utils::create_entity(world, "dummy", utils::pos(5, 10), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    utils::select(&mut app, sniper_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(mark_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 10);
    assert!(utils::order_queue_is_empty(app.world_mut(), sniper));
    assert_eq!(
        utils::health_as_u32(&app, mark),
        20,
        "a fogged target cannot be named: the attack order is refused untouched"
    );

    // A scout beside the mark lends the eyes; the same order now lands.
    utils::create_entity(app.world_mut(), "scout", utils::pos(5, 12), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(mark_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);
    // The scout's sight reveals the mark: the repeated order lands, and two
    // 10-damage shots finish its 20 health.
    utils::assert_despawned(app.world_mut(), mark);
}

#[test]
fn omniscient_player_attack_on_fogged_target_is_honored() {
    // The sight gate follows the vision the seat declares: an omniscient
    // player legitimately names what fog hides, and the seat is session
    // state, so every node resolves its commands identically. The mark
    // stands in the sniper's weapon range (8) but beyond its sight (3) —
    // the same layout the human sniper above is refused in.
    let (mut app, mark, _) = scripted_sniper_attacks_fogged_mark(AiVision::Omniscient);

    utils::assert_despawned(app.world_mut(), mark);
}

#[test]
fn fog_limited_player_attack_on_fogged_target_is_refused() {
    // A fog-limited scripted player lives under the same rule as a human:
    // its view never shows the mark, and a remembered id names nothing.
    let (mut app, mark, sniper) = scripted_sniper_attacks_fogged_mark(AiVision::Filtered);

    assert!(utils::order_queue_is_empty(app.world_mut(), sniper));
    assert_eq!(
        utils::health_as_u32(&app, mark),
        20,
        "a fogged target cannot be named: the attack order is refused untouched"
    );
}

#[test]
fn guard_refuses_fogged_ward() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let world = app.world_mut();
    let (scout, scout_id) =
        utils::create_entity(world, "scout", utils::pos(5, 5), Some(0)).unwrap();
    let (_, ward_id) = utils::create_entity(world, "dummy", utils::pos(25, 25), None).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    utils::select(&mut app, scout_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Guard {
            target: ward_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 3);

    assert!(utils::order_queue_is_empty(app.world_mut(), scout));
}

#[test]
fn follow_refuses_fogged_target() {
    // Following what the fog hides would be a live tracking beacon.
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let world = app.world_mut();
    let (scout, scout_id) =
        utils::create_entity(world, "scout", utils::pos(5, 5), Some(0)).unwrap();
    let (_, quarry_id) = utils::create_entity(world, "dummy", utils::pos(25, 25), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    utils::select(&mut app, scout_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Follow {
            target: quarry_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 3);

    assert!(utils::order_queue_is_empty(app.world_mut(), scout));
}

#[test]
fn send_to_entity_refuses_fogged_target() {
    // The smart send resolves harvest, attack, board, and the rest — every
    // one of them behind this same mouth, so an unseen mine can no more be
    // harvested than an unseen enemy attacked.
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let world = app.world_mut();
    let (scout, scout_id) =
        utils::create_entity(world, "scout", utils::pos(5, 5), Some(0)).unwrap();
    let (_, target_id) = utils::create_entity(world, "dummy", utils::pos(25, 25), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    utils::select(&mut app, scout_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: target_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 3);

    assert!(utils::order_queue_is_empty(app.world_mut(), scout));
}

#[test]
fn rally_refuses_fogged_entity_target() {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let world = app.world_mut();
    let (post, post_id) = utils::create_entity(world, "post", utils::pos(5, 5), Some(0)).unwrap();
    // The post has no eyes; the scout beside it sees for its owner.
    utils::create_entity(world, "scout", utils::pos(5, 6), Some(0)).unwrap();
    let (_, far_id) = utils::create_entity(world, "dummy", utils::pos(25, 25), Some(1)).unwrap();
    let (_, near_id) = utils::create_entity(world, "dummy", utils::pos(5, 8), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    // Fogged: the rally point stays unset.
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: post_id,
            target: Some(RallyTarget::Entity(far_id)),
        },
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        app.world().get::<RallyPointComponent>(post).unwrap().0,
        None
    );

    // Seen: the same command lands.
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: post_id,
            target: Some(RallyTarget::Entity(near_id)),
        },
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        app.world().get::<RallyPointComponent>(post).unwrap().0,
        Some(RallyTarget::Entity(near_id))
    );
}

#[test]
fn rally_on_entity_drops_when_owner_seat_loses_sight() {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let world = app.world_mut();
    let (post, post_id) = utils::create_entity(world, "post", utils::pos(5, 5), Some(0)).unwrap();
    // The post has no eyes; the scout beside it sees six cells for its owner.
    let (eyes, _) = utils::create_entity(world, "scout", utils::pos(5, 6), Some(0)).unwrap();
    let (_, quarry_id) = utils::create_entity(world, "scout", utils::pos(5, 8), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: post_id,
            target: Some(RallyTarget::Entity(quarry_id)),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        app.world().get::<RallyPointComponent>(post).unwrap().0,
        Some(RallyTarget::Entity(quarry_id))
    );

    // The rival walks its scout to the far corner, out of the local scout's
    // sight long before it arrives: the rally lapses with the sight of it.
    let due = utils::tick(&app) + utils::APPLY;
    utils::run_ticks_commanding(
        &mut app,
        40,
        1,
        due,
        vec![
            PlayerCommand::SelectById {
                id: quarry_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Move {
                target: utils::pos(25, 25),
                flush: true,
            },
        ],
    );
    assert_eq!(
        app.world().get::<RallyPointComponent>(post).unwrap().0,
        None,
        "a rally on what the owner cannot make out is dropped"
    );

    // A scout trained now is sent nowhere: it stands beside the post.
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: post_id,
            type_name: "scout".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 11);
    let recruits: Vec<Entity> = utils::owned_of_type(app.world_mut(), "scout", 0)
        .into_iter()
        .filter(|&scout| scout != eyes)
        .collect();
    let [recruit] = recruits.as_slice() else {
        panic!("the post trained exactly one scout");
    };
    assert!(utils::order_queue_is_empty(app.world_mut(), *recruit));
    utils::assert_adjacent_to_footprint(app.world_mut(), *recruit, post);
}

//
// ─── Watches ─────────────────────────────────────────────────────────────────
//

#[test]
fn watch_reveals_patch_no_entity_can_see() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let (_, station) =
        utils::create_entity(app.world_mut(), "station", utils::pos(2, 2), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(!visible(&app, 0, 20, 20), "the far cell starts dark");

    sweep_at(&mut app, station, 20, 20);
    // One tick past the command's own delay: a cast is an order, so the
    // watch is set in the order phase, which the tick's fog pass has
    // already run.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // Radius two around the aim, and nothing outside it.
    assert!(visible(&app, 0, 20, 20));
    assert!(visible(&app, 0, 20, 22));
    assert!(!visible(&app, 0, 20, 23));
}

#[test]
fn watch_lapses_exactly_when_its_duration_runs_out() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let (_, station) =
        utils::create_entity(app.world_mut(), "station", utils::pos(2, 2), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    sweep_at(&mut app, station, 20, 20);
    // The cast lands on the third tick and holds for the five ticks after
    // it: the fifth of those is the last one the patch is in sight.
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert!(visible(&app, 0, 20, 20), "the fifth tick still sees it");
    utils::run_ticks(&mut app, 1);
    assert!(!visible(&app, 0, 20, 20), "the sixth does not");
    assert_eq!(
        app.world().resource::<VisibilityGrid>().get(0, 20, 20),
        CellVisibility::Explored,
        "what it saw stays remembered"
    );
}

#[test]
fn watch_reads_its_radius_and_duration_off_caster() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let (_, observatory) =
        utils::create_entity(app.world_mut(), "observatory", utils::pos(2, 2), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    utils::cast_at(&mut app, observatory, "far_sweep", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // `scan_radius` 3: three cells out from the aim, and not four.
    assert!(visible(&app, 0, 20, 23));
    assert!(!visible(&app, 0, 20, 24));
    // `scan_time` 2: the cast lands on the third tick and holds two more.
    utils::run_ticks(&mut app, 1);
    assert!(visible(&app, 0, 20, 20), "the second tick still sees it");
    utils::run_ticks(&mut app, 1);
    assert!(!visible(&app, 0, 20, 20), "the third does not");
}

#[test]
fn watch_outlives_caster_and_is_shared_with_allies() {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(0)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(0)),
    ]);
    let (station_entity, station) =
        utils::create_entity(app.world_mut(), "station", utils::pos(2, 2), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    sweep_at(&mut app, station, 20, 20);
    // One tick past the command's own delay: a cast is an order, so the
    // watch is set in the order phase, which the tick's fog pass has
    // already run.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(visible(&app, 0, 20, 20));
    assert!(visible(&app, 1, 20, 20), "an ally reads the same patch");

    // The station goes; the sight it left does not go with it.
    ferrets_simulation::spawn::despawn_entity(
        app.world_mut(),
        station_entity,
        ferrets_simulation::events::DeathCause::Depleted,
    );
    utils::run_ticks(&mut app, 1);
    assert!(visible(&app, 0, 20, 20), "the watch stands on its own");
}

#[test]
fn run_out_watch_leaves_store() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let (_, station) =
        utils::create_entity(app.world_mut(), "station", utils::pos(2, 2), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    sweep_at(&mut app, station, 20, 20);
    // One tick past the command's own delay: a cast is an order, so the
    // watch is set in the order phase, which the tick's fog pass has
    // already run.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(
        app.world().resource::<Watches>().in_force().len(),
        1,
        "the sweep is in force"
    );

    // Five ticks of duration, and the tick that takes the last one off it
    // drops it: what the store holds is what is in force.
    utils::run_ticks(&mut app, 5);
    assert!(
        app.world().resource::<Watches>().in_force().is_empty(),
        "nothing is left in the store"
    );
}

//
// ─── Relief ──────────────────────────────────────────────────────────────────
//

#[test]
fn relief_hides_higher_ground_from_viewer_below() {
    let mut app = relief_app(&["ground"]);
    utils::create_entity(app.world_mut(), "watchman", utils::pos(4, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    // Its eyes at 1, the low ground's level: the low ground shows, the
    // plateau at 2 stays hidden, its edge (8, 5) and beyond.
    assert!(visible(&app, 0, 6, 5));
    assert!(!visible(&app, 0, 8, 5));
    assert!(!visible(&app, 0, 10, 5));
}

#[test]
fn viewer_on_high_ground_sees_low_ground() {
    let mut app = relief_app(&["ground"]);
    utils::create_entity(app.world_mut(), "watchman", utils::pos(11, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    // Its eyes at 2: the low ground at 1 shows, the cliff foot (7, 5) too.
    assert!(visible(&app, 0, 9, 5));
    assert!(visible(&app, 0, 7, 5));
    assert!(visible(&app, 0, 5, 5));
}

#[test]
fn sees_up_lifts_eyes_over_relief() {
    let mut app = relief_app(&["ground"]);
    utils::create_entity(app.world_mut(), "lookout", utils::pos(4, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    // Eyes at 1 + 1 = 2, the plateau's level.
    assert!(visible(&app, 0, 8, 5));
    assert!(visible(&app, 0, 10, 5));
}

#[test]
fn buffed_sees_up_lifts_eyes_over_relief() {
    let mut app = relief_app(&["ground"]);
    let watchman = utils::create_entity(app.world_mut(), "watchman", utils::pos(4, 5), Some(0))
        .unwrap()
        .0;
    let keen = utils::register_entity_buff(
        &mut app,
        "keen",
        EntityStatId::SEES_UP,
        ModifierOp::FlatAdd,
        "1",
        None,
    );
    utils::apply_buff(app.world_mut(), watchman, keen);
    // The fog is stamped before the tick folds the buff into the stat: the
    // second tick sees with it.
    utils::run_ticks(&mut app, 2);

    // Eyes at 1 + (0 + 1) = 2, the plateau's level.
    assert!(visible(&app, 0, 10, 5));
}

#[test]
fn relief_blocking_no_one_hides_nothing() {
    let mut app = relief_app(&[]);
    utils::create_entity(app.world_mut(), "watchman", utils::pos(4, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    assert!(visible(&app, 0, 8, 5));
    assert!(visible(&app, 0, 10, 5));
}

#[test]
fn relief_hides_its_cells_from_lower_layer_it_names() {
    let mut app = relief_app(&["deep", "ground"]);
    utils::create_entity(app.world_mut(), "diver", utils::pos(4, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    // Its eyes at 0, under the ground everywhere: the low ground at 1 is
    // above them as the plateau is, its own cell included.
    assert!(!visible(&app, 0, 4, 5));
    assert!(!visible(&app, 0, 6, 5));
    assert!(!visible(&app, 0, 10, 5));
}

#[test]
fn relief_never_hides_from_lower_layer_it_does_not_name() {
    let mut app = relief_app(&["ground"]);
    utils::create_entity(app.world_mut(), "diver", utils::pos(4, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    assert!(visible(&app, 0, 4, 5));
    assert!(visible(&app, 0, 10, 5));
}

//
// ─── Shadows on relief ───────────────────────────────────────────────────────
//

#[test]
fn rock_on_plateau_shadows_lookout_below() {
    let mut app = relief_app(&["ground"]);
    utils::create_entity(app.world_mut(), "rock", utils::pos(10, 5), None).unwrap();
    utils::create_entity(app.world_mut(), "lookout", utils::pos(4, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    // Eyes at 1 + 1 = 2 see the plateau at 2; the rock standing on it rises
    // to 2 + 1 = 3, and hides what lies behind it.
    assert!(visible(&app, 0, 10, 5));
    assert!(!visible(&app, 0, 12, 5));
}

#[test]
fn smoke_rises_from_level_of_each_cell_it_covers() {
    let mut app = relief_app(&["ground"]);
    utils::create_owned(&mut app, "smoke_pot", 5, 8, 0);
    utils::create_owned(&mut app, "smoke_pot", 10, 5, 0);
    utils::create_entity(app.world_mut(), "lookout", utils::pos(5, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, 1);

    // Eyes at 1 + 1 = 2. Over low ground the smoke's top is 1 + 1 = 2, not
    // above them: (5, 10) shows past it. Over the plateau it is 2 + 1 = 3:
    // (12, 5) lies in its shadow.
    assert!(visible(&app, 0, 5, 10));
    assert!(!visible(&app, 0, 12, 5));
}

#[test]
fn wide_viewer_across_cliff_sees_from_its_highest_cell() {
    let mut app = relief_app(&["ground"]);
    utils::create_entity(app.world_mut(), "wide_strider", utils::pos(7, 5), Some(0))
        .expect("a strider stands across the cliff");
    utils::run_ticks(&mut app, 1);

    // Its cells span columns 7 (level 1) and 8 (level 2): its eyes at the
    // higher, 2, see the plateau.
    assert!(visible(&app, 0, 11, 5));
}

//
// ─── Shadows ─────────────────────────────────────────────────────────────────
//

#[test]
fn rock_shadows_wedge_behind_it() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 6, 5);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Seen from (4.5, 5.5): the rock itself and the cells beside it show,
    // the wedge behind it widens with distance. The line to (10.5, 3.5) falls
    // a third of a cell a cell, reaching y 5.0 at x 6: it touches the rock's
    // corner (6, 5) alone and passes. The line to (9.5, 4.5) is at y 5.2 to
    // 5.0 across x 6 to 7, inside the rock.
    for (x, y) in [(6, 5), (7, 4), (7, 6), (10, 3), (10, 7)] {
        assert!(visible(&app, 0, x, y), "({x}, {y}) is seen");
    }
    for (x, y) in [(7, 5), (8, 5), (10, 5), (8, 4), (9, 4), (8, 6), (10, 6)] {
        assert!(!visible(&app, 0, x, y), "({x}, {y}) lies in the shadow");
    }
}

#[test]
fn object_never_shadows_its_own_cells() {
    let mut app = shadow_app();
    place_unowned(&mut app, "boulder", 6, 5);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The boulder's far column shows; what lies behind it does not.
    assert!(visible(&app, 0, 7, 5));
    assert!(visible(&app, 0, 7, 6));
    assert!(!visible(&app, 0, 8, 5));
    assert!(!visible(&app, 0, 8, 6));
    assert!(!visible(&app, 0, 9, 5));
}

#[test]
fn line_brushing_rock_corner_passes() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 5, 5);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // From (4.5, 5.5) to (6.5, 7.5) the line touches the rock's corner
    // (5, 6) and never enters it.
    assert!(visible(&app, 0, 6, 7));
}

#[test]
fn rocks_meeting_at_corner_close_line_between_them() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 5, 5);
    place_unowned(&mut app, "rock", 4, 6);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The same line passes through the corner (5, 6) the two rocks share.
    assert!(!visible(&app, 0, 6, 7));
}

#[test]
fn rocks_meeting_at_corner_close_rising_line_between_them() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 4, 4);
    place_unowned(&mut app, "rock", 5, 5);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // From (4.5, 5.5) to (5.5, 4.5) the line passes through the corner (5, 5)
    // the two rocks share, the other diagonal from the line above.
    assert!(!visible(&app, 0, 5, 4));
}

#[test]
fn sees_up_lifts_eyes_over_rock() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 6, 5);
    utils::create_owned(&mut app, "tall_eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The rock's top at 1 + 1 = 2, the eyes at 1 + 1 = 2: not below it.
    assert!(visible(&app, 0, 8, 5));
}

#[test]
fn wide_viewer_sees_from_corner_its_cells_share() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 7, 4);
    utils::create_owned(&mut app, "wide_eye", 3, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Seen from (5, 6), the corner of its four cells; the rock spans x 7 to 8,
    // y 4 to 5. The line to (10.5, 4.5) is at y 5.45 to 5.18 across the
    // rock's columns, below it; the line to (10.5, 3.5) is at y 5.09 to 4.64,
    // into it.
    for (x, y) in [(10, 4), (8, 5), (10, 5)] {
        assert!(visible(&app, 0, x, y), "({x}, {y}) is seen");
    }
    for (x, y) in [(8, 4), (9, 4), (8, 3), (9, 3), (10, 3)] {
        assert!(!visible(&app, 0, x, y), "({x}, {y}) lies in the shadow");
    }
}

#[test]
fn rock_shadows_wedge_west_of_viewer() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 8, 5);
    utils::create_owned(&mut app, "eye", 10, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The first wedge mirrored: seen from (10.5, 5.5), the rock two cells
    // west.
    for (x, y) in [(8, 5), (7, 4), (7, 6), (4, 3), (4, 7)] {
        assert!(visible(&app, 0, x, y), "({x}, {y}) is seen");
    }
    for (x, y) in [(7, 5), (6, 5), (4, 5), (6, 4), (5, 4), (6, 6), (4, 6)] {
        assert!(!visible(&app, 0, x, y), "({x}, {y}) lies in the shadow");
    }
}

#[test]
fn rock_shadows_wedge_north_of_viewer() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 5, 8);
    utils::create_owned(&mut app, "eye", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    // The first wedge turned a quarter: seen from (5.5, 10.5), the rock two
    // cells north.
    for (x, y) in [(5, 8), (4, 7), (6, 7), (3, 4), (7, 4)] {
        assert!(visible(&app, 0, x, y), "({x}, {y}) is seen");
    }
    for (x, y) in [(5, 7), (5, 6), (5, 4), (4, 6), (4, 5), (6, 6), (6, 4)] {
        assert!(!visible(&app, 0, x, y), "({x}, {y}) lies in the shadow");
    }
}

#[test]
fn rock_shadows_boulder_behind_it() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 6, 5);
    place_unowned(&mut app, "boulder", 8, 5);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The rock hides the boulder's near cells; the boulder never hides its
    // own, but another object's cells are not its own.
    assert!(!visible(&app, 0, 8, 5));
    assert!(!visible(&app, 0, 8, 6));
    assert!(!visible(&app, 0, 9, 5));
}

#[test]
fn rock_near_edge_of_sight_shadows_cell_behind_it() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 11, 5);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Seven cells out and eight: the rock one cell inside the eye's sight of
    // eight, the cell it hides on its edge.
    assert!(visible(&app, 0, 11, 5));
    assert!(!visible(&app, 0, 12, 5));
}

#[test]
fn tall_viewer_shadowed_at_west_edge_of_its_sight() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 9, 5);
    utils::create_owned(&mut app, "column", 10, 4, 0);
    utils::run_ticks(&mut app, 1);

    // Seen from (10.5, 6.0), the middle of cells (10, 4) to (10, 7), with a
    // sight of one: the line to (9.5, 4.5) is at y 5.25 at x 10, crossing the
    // rock in the sight's west column.
    assert!(visible(&app, 0, 9, 6));
    assert!(!visible(&app, 0, 9, 4));
}

#[test]
fn tall_viewer_shadowed_at_east_edge_of_its_sight() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 11, 5);
    utils::create_owned(&mut app, "column", 10, 4, 0);
    utils::run_ticks(&mut app, 1);

    // The same, mirrored: the line to (11.5, 4.5) crosses the rock in the
    // sight's east column.
    assert!(visible(&app, 0, 11, 6));
    assert!(!visible(&app, 0, 11, 4));
}

#[test]
fn field_blocks_cells_of_object_it_covers() {
    let mut app = shadow_app();
    place_unowned(&mut app, "boulder", 8, 4);
    utils::create_owned(&mut app, "smoke_pot", 8, 7, 1);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The smoke reaches the boulder's cell (8, 5): the boulder never hides
    // its own (9, 5), but the smoke on its cell does.
    assert!(!visible(&app, 0, 9, 5));
}

#[test]
fn boundless_sight_reaches_no_further_than_map() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "far_eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Two billion cells of sight, held to the map's 32 + 32: every corner
    // shows, and the pass walks a square reaching 64 cells round the eye
    // rather than two billion.
    for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31)] {
        assert!(visible(&app, 0, x, y), "({x}, {y}) is seen");
    }
}

#[test]
fn dying_rock_shadows_until_gone() {
    let mut app = shadow_app();
    let cairn = utils::create_entity(app.world_mut(), "cairn", utils::pos(6, 5), None)
        .unwrap()
        .0;
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);
    spawn::destroy_entity(app.world_mut(), cairn);
    utils::run_ticks(&mut app, 2);

    // Its body stands through ten ticks of dying, and blocks while it does.
    assert!(entity_def::dying(app.world(), cairn));
    assert!(!visible(&app, 0, 8, 5));

    // Fifteen more ticks, past its ten of dying: the body is gone, and so is
    // its shadow.
    utils::run_ticks(&mut app, 15);
    utils::assert_despawned(app.world_mut(), cairn);
    assert!(visible(&app, 0, 8, 5));
}

#[test]
fn field_blocking_enemies_shadows_enemy_sight() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "screen_pot", 10, 5, 1);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The screen covers (8, 5) to (12, 5) along the row and blocks player
    // 1's enemies.
    assert!(visible(&app, 0, 8, 5));
    assert!(!visible(&app, 0, 12, 5));
}

#[test]
fn field_blocking_enemies_never_shadows_its_coverers_sight() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "screen_pot", 10, 5, 1);
    utils::create_owned(&mut app, "eye", 4, 5, 1);
    utils::run_ticks(&mut app, 1);

    assert!(visible(&app, 1, 12, 5));
}

#[test]
fn ward_shadows_enemy_sight() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "ward", 6, 5, 1);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    assert!(!visible(&app, 0, 8, 5));
}

#[test]
fn ward_never_shadows_its_owners_sight() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "ward", 6, 5, 0);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    assert!(visible(&app, 0, 8, 5));
}

#[test]
fn smoke_shows_only_its_near_edge_from_outside() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "smoke_pot", 10, 5, 1);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The smoke covers (8, 5) to (12, 5) along the row.
    assert!(visible(&app, 0, 8, 5));
    assert!(!visible(&app, 0, 9, 5));
    assert!(!visible(&app, 0, 12, 5));
}

#[test]
fn smoke_shows_only_neighbors_from_inside() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "smoke_pot", 8, 5, 1);
    utils::create_owned(&mut app, "eye", 9, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The smoke covers (6, 5) to (10, 5) along the row, the eye's own cell
    // among them, and a cell around each of its row's neighbors.
    assert!(visible(&app, 0, 8, 5));
    assert!(visible(&app, 0, 10, 5));
    assert!(visible(&app, 0, 9, 6));
    assert!(!visible(&app, 0, 7, 5));
    assert!(!visible(&app, 0, 11, 5));
    // A diagonal neighbor stays hidden: the line to (10, 6) passes the corner
    // (10, 6) its two smoked orthogonal neighbors share.
    assert!(!visible(&app, 0, 10, 6));
}

#[test]
fn flier_sees_over_smoke_on_ground() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "smoke_pot", 10, 5, 1);
    utils::create_owned(&mut app, "flier", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The smoke's top on the ground at 1 + 1 = 2, the flier's eyes at 2.
    assert!(visible(&app, 0, 12, 5));
}

#[test]
fn haze_spread_on_air_blinds_flier() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "haze_pot", 10, 5, 1);
    utils::create_owned(&mut app, "flier", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // On the air the haze's top is at 2 + 1 = 3, above the flier's eyes.
    assert!(visible(&app, 0, 8, 5));
    assert!(!visible(&app, 0, 12, 5));
}

#[test]
fn watching_field_reveals_what_shadow_hides() {
    let mut app = shadow_app();
    place_unowned(&mut app, "rock", 6, 5);
    utils::create_owned(&mut app, "flare_post", 10, 5, 0);
    utils::create_owned(&mut app, "eye", 4, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The flare covers (8, 5) to (12, 5) along the row; (7, 5) it leaves to
    // the rock's shadow.
    assert!(visible(&app, 0, 9, 5));
    assert!(!visible(&app, 0, 7, 5));
}

//
// ─── Field watches ───────────────────────────────────────────────────────────
//

#[test]
fn field_watch_reveals_each_patch_to_its_own_coverer() {
    let mut app = shadow_app();
    utils::create_owned(&mut app, "lamp", 10, 10, 0);
    utils::create_owned(&mut app, "lamp", 13, 10, 1);
    utils::run_ticks(&mut app, 1);

    // Each glow reaches two cells: player 0's alone covers (8, 10), player
    // 1's alone (15, 10), and both cover (11, 10) and (12, 10). Neither lamp
    // sees of its own, and the two players are enemies.
    assert!(visible(&app, 0, 8, 10));
    assert!(!visible(&app, 1, 8, 10));
    assert!(visible(&app, 1, 15, 10));
    assert!(!visible(&app, 0, 15, 10));
    assert!(visible(&app, 0, 12, 10));
    assert!(visible(&app, 1, 11, 10));
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Seats a scripted sniper (player 1, with `vision`) against a human's dummy
/// mark placed in the sniper's weapon range but beyond its sight, has the
/// script select the sniper and attack the mark by id, and runs long enough
/// for an honored order to finish the mark's 20 health in two 10-damage
/// shots. Returns the app with the mark's and the sniper's entities.
fn scripted_sniper_attacks_fogged_mark(vision: AiVision) -> (App, Entity, Entity) {
    let mut app = fog_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision,
                detection: AiDetection::Detectors,
            },
            None,
            None,
        ),
    ]);
    let world = app.world_mut();
    let (sniper, sniper_id) =
        utils::create_entity(world, "sniper", utils::pos(5, 5), Some(1)).unwrap();
    let (mark, mark_id) = utils::create_entity(world, "dummy", utils::pos(5, 10), Some(0)).unwrap();
    utils::run_ticks_commanding(
        &mut app,
        25,
        1,
        utils::APPLY,
        vec![
            PlayerCommand::SelectById {
                id: sniper_id,
                mode: SelectMode::Replace,
            },
            PlayerCommand::Attack {
                target: AttackTarget::Entity(mark_id),
                flush: true,
            },
        ],
    );
    (app, mark, sniper)
}

/// Casts the station's sweep at `(x, y)`.
fn sweep_at(app: &mut App, caster: SimulationId, x: u32, y: u32) {
    utils::cast_at(app, caster, "sweep", x, y);
}

/// Whether the local-team vision of `player` covers `(x, y)`.
fn visible(app: &App, player: PlayerId, x: u32, y: u32) -> bool {
    let world = app.world();
    world
        .resource::<VisibilityGrid>()
        .is_visible_to(world.resource::<GameSession>(), player, x, y)
}

/// `player`'s team-combined knowledge of `(x, y)`.
fn cell_state(app: &App, player: PlayerId, x: u32, y: u32) -> CellVisibility {
    let world = app.world();
    world
        .resource::<VisibilityGrid>()
        .visibility_to(world.resource::<GameSession>(), player, x, y)
}

/// App with fog content: a wide-eyed `scout`, a far-sighted `sniper` whose reach
/// exceeds its vision, and a defenceless `dummy`. Session started.
fn fog_app(slots: Vec<PlayerSlot>) -> App {
    let mut app = utils::make_app(slots);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            utils::walker("scout", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_sight_range(6)
                // Trainable only so the post below validates; nothing trains
                // one in these tests.
                .with_train_time(10),
        );
        registry.register(
            utils::walker("sniper", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(30))
                .with_dying(1, [])
                .with_attack(utils::weapon(utils::GROUND), 10, 8, 8, 2, 1)
                .with_sight_range(3),
        );
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(3, 3),
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(200))
                .with_dying(1, [])
                .with_sight_range(2),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_sight_range(3),
        );
        // A trainer with no eyes of its own, for the rally-target gate: what
        // its owner sees near it comes from the units standing around.
        registry.register(
            EntityTypeDef::new("post")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(50))
                .with_dying(1, [])
                .with_trainer(["scout"]),
        );
        // A station that watches a patch of map from afar: the sight it leaves
        // behind belongs to no entity, so it lasts its stated ticks whatever
        // becomes of the caster.
        let sweep = registry.register_skill(
            "sweep",
            SkillDef {
                cooldown: 4,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Watch {
                        at: Spot::Cell {
                            reach: Reach::Wherever,
                        },
                        radius: Quantity::Constant(2),
                        duration: Quantity::Constant(5),
                        detection: Detection::Blind,
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("station")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(50))
                .with_dying(1, [])
                .with_sight_range(2)
                .with_skills([sweep]),
        );
        // The same sweep reading its reach and its time off the caster.
        let scan_radius = registry.register_entity_stat("scan_radius", FixedU64::ZERO);
        let scan_time = registry.register_entity_stat("scan_time", FixedU64::ONE);
        let far_sweep = registry.register_skill(
            "far_sweep",
            SkillDef {
                cooldown: 4,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Watch {
                        at: Spot::Cell {
                            reach: Reach::Wherever,
                        },
                        radius: Quantity::Stat(scan_radius),
                        duration: Quantity::Stat(scan_time),
                        detection: Detection::Blind,
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("observatory")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(50))
                .with_dying(1, [])
                .with_sight_range(2)
                .with_stat(scan_radius, FixedU64::from_num(3))
                .with_stat(scan_time, FixedU64::from_num(2))
                .with_skills([far_sweep]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    utils::start_game(&mut app);
    app
}

/// One human on a 16×16 land map: the ground spans levels 1 to 3, low at 1
/// to column 7 and a plateau at 2 from column 8, a cliff band rising east
/// across columns 7 and 8; under it a `deep` layer flat at 0. The ground's
/// relief blocks the sight of the layers `ground_relief_blocks` names.
///
/// - `watchman` is a static 1×1 ground body seeing 8 cells, and `lookout` the
///   same seeing one level up;
/// - `diver` is a static 1×1 body on the deep layer seeing 8 cells;
/// - `wide_strider` is a 2×2 mover striding cliffs, seeing 8 cells;
/// - `rock` is a static 1×1 ground body blocking everyone's sight one level
///   above where it stands, and `smoke_pot` a static 1×1 one spreading
///   `smoke` a cell around it, blocking everyone's sight one level above each
///   cell it covers.
fn relief_app(ground_relief_blocks: &[&str]) -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let mut registry = ContentRegistry::default();
    let deep = registry.register_layer("deep", LayerLevel::At(0), &[]);
    let ground = registry.register_layer(
        "ground",
        LayerLevel::Range { from: 1, to: 3 },
        ground_relief_blocks,
    );
    registry.register_terrain("land");
    let walking = registry.register_domain(
        "walking",
        DomainDef::new(ground, Terrains::Any, Some(CliffCrossing::Ramp)),
    );
    let striding = registry.register_domain(
        "striding",
        DomainDef::new(ground, Terrains::Any, Some(CliffCrossing::Cross)),
    );
    let delving = registry.register_domain("delving", DomainDef::new(deep, Terrains::Any, None));
    for (name, domain, layer, sees_up) in [
        ("watchman", walking, ground, FixedU64::ZERO),
        ("lookout", walking, ground, FixedU64::ONE),
        ("diver", delving, deep, FixedU64::ZERO),
    ] {
        registry.register(
            EntityTypeDef::new(name)
                .with_location(domain, CellSize::ONE, Solidity::Solid, layer)
                .with_pool(Pool::health(20))
                .with_sight_range(8)
                .with_stat(EntityStatId::SEES_UP, sees_up),
        );
    }
    registry.register(
        EntityTypeDef::new("wide_strider")
            .with_location(striding, CellSize::new(2, 2), Solidity::Solid, ground)
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("1"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::health(20))
            .with_sight_range(8),
    );
    registry.register(
        EntityTypeDef::new("rock")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(20))
            .with_blocks_sight(SightBlock::new(1, Affiliation::Anyone)),
    );
    let smoke = registry.register_field(
        "smoke",
        FieldDef::new(
            Terrains::Any,
            ground.into(),
            FieldDecay::Instant,
            None,
            Some(SightBlock::new(1, Affiliation::Anyone)),
            Detection::Blind,
        ),
    );
    registry.register(
        EntityTypeDef::new("smoke_pot")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(20))
            .with_field_sources([FieldSourceDef::new(
                smoke,
                1,
                FieldGrowth::Instant,
                Emission::Nothing,
                Emission::Nothing,
            )]),
    );
    registry.validate();

    let mut data = MapData::new("relief", Projection::Isometric, 16, 16, "land");
    data.add_relief("ground", 1);
    for y in 0..16 {
        for x in 8..16 {
            data.set_cell_level("ground", (x, y), 2);
        }
        for x in [7, 8] {
            data.set_slope("ground", (x, y), Slope::Cliff(Direction::East));
        }
    }
    let live = Map::from_data(&data, &registry);
    app.insert_resource(registry);
    map::install_map(app.world_mut(), live);
    utils::start_game(&mut app);
    app
}

/// Two hostile humans on a 32×32 land map, the ground flat at 1 and the air
/// at 2:
///
/// - `eye` is a static 1×1 ground body seeing 8 cells, `tall_eye` the same
///   seeing one level up, `wide_eye` a static 2×2 one, `column` a static 1×4
///   one seeing a cell around it, `far_eye` a 1×1 one seeing two billion
///   cells, and `flier` a static 1×1 body on the air;
/// - `rock` is a static 1×1 body blocking everyone's sight one level above
///   it, `boulder` a 2×2 one, and `ward` a 1×1 one blocking enemies only;
///   `cairn` blocks one level up through ten ticks of dying;
/// - `smoke_pot` spreads `smoke` two cells around it, blocking everyone's
///   sight one level above the ground, `haze_pot` spreads `haze`, the same
///   on the ground and the air, `screen_pot` a `screen` blocking only its
///   coverers' enemies, `flare_post` a `flare` watched for anyone, and `lamp`
///   a `glow` watched for whoever covers it.
fn shadow_app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut grid = NavGrid::new(32, 32, |_| utils::LAND);
        grid.add_flat_layer(utils::GROUND, 1);
        grid.add_flat_layer(utils::AIR, 2);
        map::install_map(
            app.world_mut(),
            Map::new(
                "test",
                Projection::Isometric,
                MovementModel::Cell,
                grid,
                vec![],
            ),
        );
    }
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        utils::register_air(&mut registry);
        let body = |name: &str, domain, layer, side| {
            EntityTypeDef::new(name)
                .with_location(domain, CellSize::new(side, side), Solidity::Solid, layer)
                .with_pool(Pool::health(50))
        };
        for (name, side, sees_up) in [
            ("eye", 1, FixedU64::ZERO),
            ("tall_eye", 1, FixedU64::ONE),
            ("wide_eye", 2, FixedU64::ZERO),
        ] {
            registry.register(
                body(name, utils::GROUND_DOMAIN, utils::GROUND, side)
                    .with_sight_range(8)
                    .with_stat(EntityStatId::SEES_UP, sees_up),
            );
        }
        registry.register(
            EntityTypeDef::new("column")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(1, 4),
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(50))
                .with_sight_range(1),
        );
        registry.register(
            body("far_eye", utils::GROUND_DOMAIN, utils::GROUND, 1).with_sight_range(2_000_000_000),
        );
        registry.register(body("flier", utils::AIR_DOMAIN, utils::AIR, 1).with_sight_range(8));
        for (name, side, of) in [
            ("rock", 1, Affiliation::Anyone),
            ("boulder", 2, Affiliation::Anyone),
            ("ward", 1, Affiliation::Enemy),
        ] {
            registry.register(
                body(name, utils::GROUND_DOMAIN, utils::GROUND, side)
                    .with_blocks_sight(SightBlock::new(1, of)),
            );
        }
        registry.register(
            body("cairn", utils::GROUND_DOMAIN, utils::GROUND, 1)
                .with_dying(10, [])
                .with_blocks_sight(SightBlock::new(1, Affiliation::Anyone)),
        );
        let smoke = registry.register_field(
            "smoke",
            FieldDef::new(
                Terrains::Any,
                utils::GROUND.into(),
                FieldDecay::Instant,
                None,
                Some(SightBlock::new(1, Affiliation::Anyone)),
                Detection::Blind,
            ),
        );
        let haze = registry.register_field(
            "haze",
            FieldDef::new(
                Terrains::Any,
                utils::GROUND | utils::AIR,
                FieldDecay::Instant,
                None,
                Some(SightBlock::new(1, Affiliation::Anyone)),
                Detection::Blind,
            ),
        );
        let flare = registry.register_field(
            "flare",
            FieldDef::new(
                Terrains::Any,
                utils::GROUND.into(),
                FieldDecay::Instant,
                Some(FieldWatch::new(Affiliation::Anyone)),
                None,
                Detection::Blind,
            ),
        );
        let screen = registry.register_field(
            "screen",
            FieldDef::new(
                Terrains::Any,
                utils::GROUND.into(),
                FieldDecay::Instant,
                None,
                Some(SightBlock::new(1, Affiliation::Enemy)),
                Detection::Blind,
            ),
        );
        let glow = registry.register_field(
            "glow",
            FieldDef::new(
                Terrains::Any,
                utils::GROUND.into(),
                FieldDecay::Instant,
                Some(FieldWatch::new(Affiliation::Own)),
                None,
                Detection::Blind,
            ),
        );
        for (name, field) in [
            ("smoke_pot", smoke),
            ("haze_pot", haze),
            ("screen_pot", screen),
            ("flare_post", flare),
            ("lamp", glow),
        ] {
            registry.register(
                body(name, utils::GROUND_DOMAIN, utils::GROUND, 1).with_field_sources([
                    FieldSourceDef::new(
                        field,
                        2,
                        FieldGrowth::Instant,
                        Emission::Nothing,
                        Emission::Nothing,
                    ),
                ]),
            );
        }
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    utils::start_game(&mut app);
    app
}

/// Places an unowned `type_name` anchored at `(x, y)`.
fn place_unowned(app: &mut App, type_name: &str, x: u32, y: u32) {
    utils::create_entity(app.world_mut(), type_name, utils::pos(x, y), None)
        .unwrap_or_else(|| panic!("{type_name} fits at ({x}, {y})"));
}
