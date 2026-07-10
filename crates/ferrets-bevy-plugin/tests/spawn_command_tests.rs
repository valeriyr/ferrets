//! The `Spawn` command creates an entity through the deterministic command pipeline.

mod utils;

use ferrets_simulation::command::PlayerCommand;

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
