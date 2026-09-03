//! Move order: passable entities path through the world without claiming cells.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{command::PlayerCommand, map::Map, spawn};

#[test]
fn passable_entities_never_claim_cells() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (ghost, ghost_id) =
        spawn::create_entity(world, "ghost", utils::pos(5, 5), Some(0)).unwrap();

    // The ghost stands at (5,5) without claiming the cell, so a solid soldier
    // can be placed right on top of it.
    assert!(
        !world
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, CellPos::new(5, 5))
    );
    let (_, _) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    assert!(
        world
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, CellPos::new(5, 5))
    );

    // The ghost walks away — it collides with the world for pathing, but its
    // crossings leave no occupancy trail.
    utils::select(&mut app, ghost_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(9, 5),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::cell_of(app.world_mut(), ghost), CellPos::new(9, 5));
    assert!(
        !app.world_mut()
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, CellPos::new(9, 5))
    );
}
