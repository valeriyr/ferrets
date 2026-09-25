//! Fog of war end-to-end: sight reveals cells, exploration is sticky, allied
//! vision is shared, combat waits for vision, and the AI view filters fogged
//! enemies only when the brain is fog-limited.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::game_view;
use ferrets_content::{
    detection::Detection,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    registry::ContentRegistry,
    skills::{Casting, EntityCastEffect, EntityCastTarget, Reach, SkillCaster, SkillDef},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode, SkillCasterRef, SkillTarget},
    components::rally::{RallyPointComponent, RallyTarget},
    order::AttackTarget,
    session::{
        GameSession, ai_detection::AiDetection, ai_vision::AiVision, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
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
        utils::health(&app, mark),
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
        utils::health(&app, mark),
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
    utils::use_skill(
        app,
        "sweep",
        SkillCasterRef::Entity(caster),
        Some(SkillTarget::Position(utils::pos(x, y))),
    );
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
            utils::walker("scout", utils::GROUND)
                .with_health(20)
                .with_dying(1, [])
                .with_sight_range(6)
                // Trainable only so the post below validates; nothing trains
                // one in these tests.
                .with_train_time(10),
        );
        registry.register(
            utils::walker("sniper", utils::GROUND)
                .with_health(30)
                .with_dying(1, [])
                .with_attack(utils::weapon(utils::GROUND), 10, 8, 8, 2, 1)
                .with_sight_range(3),
        );
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(utils::GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_health(200)
                .with_dying(1, [])
                .with_sight_range(2),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(1, [])
                .with_sight_range(3),
        );
        // A trainer with no eyes of its own, for the rally-target gate: what
        // its owner sees near it comes from the units standing around.
        registry.register(
            EntityTypeDef::new("post")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50)
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
                    target: EntityCastTarget::Position,
                    reach: Reach::Wherever,
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Watch {
                        radius: 2,
                        duration: 5,
                        detection: Detection::Blind,
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("station")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50)
                .with_dying(1, [])
                .with_sight_range(2)
                .with_skills([sweep]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
