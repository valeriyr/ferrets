//! Fog of war end-to-end: sight reveals cells, exploration is sticky, allied
//! vision is shared, combat waits for vision, and the AI view filters fogged
//! enemies only when the brain is fog-limited.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::game_view;
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_script::ai::AiVision;
use ferrets_simulation::{
    command::PlayerCommand,
    content::{entity_type_def::EntityTypeDef, location::Solidity, registry::ContentRegistry},
    session::{
        GameSession,
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
    spawn::spawn_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(visible(&app, 0, 5, 5));
    assert!(visible(&app, 0, 5, 10)); // 5 cells away, within sight 6
    assert!(!visible(&app, 0, 5, 15)); // 10 cells away, beyond sight
}

#[test]
fn exploration_persists_after_unit_moves_away() {
    let mut app = fog_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let (_, scout) =
        spawn::spawn_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
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
    spawn::spawn_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(1)).unwrap();
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
    spawn::spawn_entity(app.world_mut(), "sniper", utils::pos(5, 5), Some(0)).unwrap();
    let (dummy, _) =
        spawn::spawn_entity(app.world_mut(), "dummy", utils::pos(11, 5), Some(1)).unwrap();
    utils::run_ticks(&mut app, 20);
    assert!(
        app.world().get_entity(dummy).is_ok(),
        "dummy was attacked while unseen"
    );

    // A scout reveals the dummy to the team; now the sniper engages it.
    spawn::spawn_entity(app.world_mut(), "scout", utils::pos(9, 5), Some(0)).unwrap();
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
    spawn::spawn_entity(app.world_mut(), "scout", utils::pos(5, 5), Some(0)).unwrap();
    spawn::spawn_entity(app.world_mut(), "dummy", utils::pos(25, 25), Some(1)).unwrap();
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
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(1, None)
                .with_sight_range(6),
        );
        registry.register(
            EntityTypeDef::new("sniper")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(1, None)
                .with_attack(10, 8, 8, 2, 1)
                .with_sight_range(3),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(1, None)
                .with_sight_range(3),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
