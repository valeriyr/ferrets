//! Fog of war end-to-end: sight reveals cells, exploration is sticky, allied
//! vision is shared, combat waits for vision, and the AI view filters fogged
//! enemies only when the brain is fog-limited.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::game_view;
use ferrets_content::{
    attack::{AttackDef, Delivery, Weapon},
    entity_type_def::EntityTypeDef,
    location::Solidity,
    registry::ContentRegistry,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::rally::{RallyPointComponent, RallyTarget},
    input::{InputFrames, PlayerFrame},
    order::AttackTarget,
    session::{
        GameSession,
        ai_vision::AiVision,
        player_slot::{PlayerId, PlayerSlot},
        player_type::PlayerType,
    },
    spawn,
    visibility::{CellVisibility, VisibilityGrid},
};

//
// ─── Vision ──────────────────────────────────────────────────────────────────
//

#[test]
fn unit_reveals_cells_within_sight() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    spawn::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(visible(&app, 0, 5, 5));
    assert!(visible(&app, 0, 5, 10)); // 5 cells away, within sight 6
    assert!(!visible(&app, 0, 5, 15)); // 10 cells away, beyond sight
}

#[test]
fn exploration_persists_after_unit_moves_away() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let (_, scout) =
        spawn::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
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
    spawn::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(1)).unwrap();
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
    spawn::create_entity(app.world_mut(), "sniper", utils::pos(5, 5), Some(0)).unwrap();
    let (dummy, _) =
        spawn::create_entity(app.world_mut(), "dummy", utils::pos(11, 5), Some(1)).unwrap();
    utils::run_ticks(&mut app, 20);
    assert!(
        app.world().get_entity(dummy).is_ok(),
        "dummy was attacked while unseen"
    );

    // A scout reveals the dummy to the team; now the sniper engages it.
    spawn::create_entity(app.world_mut(), "scout", utils::pos(9, 5), Some(0)).unwrap();
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
    spawn::create_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
    spawn::create_entity(app.world_mut(), "dummy", utils::pos(25, 25), Some(1)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    let world = app.world();
    let fog_limited = game_view(world, 0, "human", AiVision::Filtered);
    let omniscient = game_view(world, 0, "human", AiVision::Omniscient);
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
        spawn::create_entity(world, "sniper", utils::pos(5, 5), Some(0)).unwrap();
    let (mark, mark_id) = spawn::create_entity(world, "dummy", utils::pos(5, 10), Some(1)).unwrap();
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
    spawn::create_entity(app.world_mut(), "scout", utils::pos(5, 12), Some(0)).unwrap();
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
        spawn::create_entity(world, "scout", utils::pos(5, 5), Some(0)).unwrap();
    let (_, ward_id) = spawn::create_entity(world, "dummy", utils::pos(25, 25), None).unwrap();
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
        spawn::create_entity(world, "scout", utils::pos(5, 5), Some(0)).unwrap();
    let (_, quarry_id) = spawn::create_entity(world, "dummy", utils::pos(25, 25), Some(1)).unwrap();
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
        spawn::create_entity(world, "scout", utils::pos(5, 5), Some(0)).unwrap();
    let (_, target_id) = spawn::create_entity(world, "dummy", utils::pos(25, 25), Some(1)).unwrap();
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
    let (post, post_id) = spawn::create_entity(world, "post", utils::pos(5, 5), Some(0)).unwrap();
    // The post has no eyes; the scout beside it sees for its owner.
    spawn::create_entity(world, "scout", utils::pos(5, 6), Some(0)).unwrap();
    let (_, far_id) = spawn::create_entity(world, "dummy", utils::pos(25, 25), Some(1)).unwrap();
    let (_, near_id) = spawn::create_entity(world, "dummy", utils::pos(5, 8), Some(1)).unwrap();
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
        PlayerSlot::occupied(1, PlayerType::Ai { vision }, None, None),
    ]);
    let world = app.world_mut();
    let (sniper, sniper_id) =
        spawn::create_entity(world, "sniper", utils::pos(5, 5), Some(1)).unwrap();
    let (mark, mark_id) = spawn::create_entity(world, "dummy", utils::pos(5, 10), Some(0)).unwrap();
    run_ticks_commanding(
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

/// Runs `ticks` fixed updates feeding idle frames the way `utils::run_ticks`
/// does, except that `player`'s frame at tick `at` carries `commands` — the
/// one way to issue commands as a non-local player, whose input never flows
/// through `PendingInput`.
fn run_ticks_commanding(
    app: &mut App,
    ticks: u32,
    player: PlayerId,
    at: u32,
    commands: Vec<PlayerCommand>,
) {
    let mut commands = Some(commands);
    for _ in 0..ticks {
        let world = app.world_mut();
        let (current_tick, local_player, players) = {
            let session = world.resource::<GameSession>();
            let players: Vec<PlayerId> = session.slots().iter().map(|slot| slot.id()).collect();
            (session.tick(), session.local_player(), players)
        };
        for other in players {
            if Some(other) == local_player {
                continue;
            }
            let frame = match commands.take_if(|_| other == player && current_tick == at) {
                Some(commands) => PlayerFrame {
                    player,
                    tick: current_tick,
                    commands,
                },
                None => PlayerFrame::idle(other, current_tick),
            };
            world.resource_mut::<InputFrames>().push_frame(frame);
        }
        world.run_schedule(FixedUpdate);
    }
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
            EntityTypeDef::new("scout")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(1, None)
                .with_sight_range(6)
                // Trainable only so the post below validates; nothing trains
                // one in these tests.
                .with_train_time(10),
        );
        registry.register(
            EntityTypeDef::new("sniper")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(1, None)
                .with_attack(
                    AttackDef::new(Weapon::new(utils::GROUND, Delivery::Instant, None)),
                    10,
                    8,
                    8,
                    2,
                    1,
                )
                .with_sight_range(3),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(1, None)
                .with_sight_range(3),
        );
        // A trainer with no eyes of its own, for the rally-target gate: what
        // its owner sees near it comes from the units standing around.
        registry.register(
            EntityTypeDef::new("post")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50)
                .with_dying(1, None)
                .with_trainer(["scout"]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
