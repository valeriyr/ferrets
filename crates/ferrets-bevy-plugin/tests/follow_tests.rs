//! Follow order: keeping a moving target within range.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{command::PlayerCommand, spawn};

#[test]
fn follow_tracks_moving_target() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (follower, follower_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (leader, leader_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(8, 5), Some(0)).unwrap();

    // A friendly target without collect/store intent resolves to a follow.
    utils::select(&mut app, follower_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: leader_id,
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 7);
    {
        let world = app.world_mut();
        assert!(utils::within(world, follower, leader, 1));
    }

    // The leader walks away; the follower keeps up.
    utils::select(&mut app, leader_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(14, 9),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 20);
    {
        let world = app.world_mut();
        assert!(utils::cell_of(world, leader) == CellPos::new(14, 9));
        assert!(utils::within(world, follower, leader, 1));
    }

    // The follow order keeps running for as long as the target is alive.
    let world = app.world_mut();
    assert!(!utils::order_queue_is_empty(world, follower));
}
