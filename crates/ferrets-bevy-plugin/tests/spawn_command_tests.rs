//! The `Spawn` command creates an entity through the deterministic command pipeline.

mod utils;

use bevy::prelude::*;
use ferrets_content::{entity_type_def::EntityTypeDef, location::Solidity};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_simulation::{
    command::PlayerCommand, components::entity_info::EntityInfoComponent,
    movement_model::MovementModel,
};

#[test]
fn spawn_command_creates_entity() {
    let mut app = utils::orders_app();
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 0);

    utils::push_command(
        &mut app,
        PlayerCommand::Spawn {
            type_name: "soldier".into(),
            position: utils::pos(10, 10),
        },
    );

    // 2-tick input latency: the command executes on the 3rd tick.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 1);
}

//
// ─── Map edges ────────────────────────────────────────────────────────────────
//

#[test]
fn spawn_near_origin_is_held_on_map_under_cell_model() {
    // A 2×2 wagon at (0.3, 6) would reach 0.3 − 1 before the origin: held at
    // x 2/2 = 1, it is anchored at (0, 5) — low edges 1 − 1 = 0 and
    // 6 − 1 = 5 — and lines up there, its middle at (0 + 1, 5 + 1).
    let mut app = utils::orders_app();
    let wagon = spawn(&mut app, "wagon", utils::point("0.3", "6"));
    assert_eq!(utils::anchor_of(app.world(), wagon), CellPos::new(0, 5));
    assert_eq!(
        utils::position_of(app.world(), wagon),
        utils::point("1", "6")
    );
}

#[test]
fn spawn_past_far_edge_is_held_on_map_under_continuous_model() {
    // On the 32-wide map a 2×2 at x 31.7 would reach 31.7 + 1 past the edge:
    // held at 32 − 1 = 31. The continuous model keeps its y of 6.3 off the
    // lattice.
    let mut app = utils::continuous_orders_app();
    let wagon = spawn(&mut app, "wagon", utils::point("31.7", "6.3"));
    assert_eq!(
        utils::position_of(app.world(), wagon),
        utils::point("31", "6.3")
    );
}

#[test]
fn standing_body_spawned_at_point_lines_up_on_its_cells() {
    // A 2×2 barracks at (6.3, 6.7) on a continuous map: low edges 5.3 and 5.7
    // round to (5, 6), and a standing body lines up on the cells it holds, its
    // middle at (5 + 1, 6 + 1) — not left 0.3 off them as a mover would be.
    let mut app = utils::continuous_orders_app();
    let barracks = spawn(&mut app, "barracks", utils::point("6.3", "6.7"));
    assert_eq!(utils::anchor_of(app.world(), barracks), CellPos::new(5, 6));
    assert_eq!(
        utils::position_of(app.world(), barracks),
        utils::point("6", "7")
    );
}

#[test]
fn spawn_off_map_is_ignored() {
    // The map is 32 cells wide: x 32.5 lies past its last column.
    let mut app = utils::orders_app();
    utils::push_command(
        &mut app,
        PlayerCommand::Spawn {
            type_name: "soldier".into(),
            position: utils::point("32.5", "6"),
        },
    );

    // 2-tick input latency: the command executes on the 3rd tick.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 0);
}

#[test]
fn spawn_wider_than_map_is_ignored() {
    // The map is 32 cells across: a 40×40 colossus has no place on it.
    let mut app = utils::dry_orders_app(MovementModel::Cell, &[], |registry, _| {
        registry.register(EntityTypeDef::new("colossus").with_location(
            utils::GROUND_DOMAIN,
            CellSize::new(40, 40),
            Solidity::Solid,
            utils::GROUND,
        ));
    });
    utils::push_command(
        &mut app,
        PlayerCommand::Spawn {
            type_name: "colossus".into(),
            position: utils::pos(10, 10),
        },
    );

    // 2-tick input latency: the command executes on the 3rd tick.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "colossus"), 0);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Spawns one `type_name` at `position` through the command pipeline and
/// returns it.
fn spawn(app: &mut App, type_name: &str, position: FixedUVec2) -> Entity {
    utils::push_command(
        app,
        PlayerCommand::Spawn {
            type_name: type_name.into(),
            position,
        },
    );
    // 2-tick input latency: the command executes on the 3rd tick.
    utils::run_ticks(app, 3);
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &EntityInfoComponent)>();
    let spawned: Vec<Entity> = query
        .iter(world)
        .filter(|(_, info)| info.type_name() == type_name)
        .map(|(entity, _)| entity)
        .collect();
    assert_eq!(spawned.len(), 1, "one {type_name} spawned");
    spawned[0]
}
