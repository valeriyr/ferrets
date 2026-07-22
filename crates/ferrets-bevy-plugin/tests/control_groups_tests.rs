//! Control groups through the command pipeline: assign, append, recall, and the
//! pruning of destroyed members.

mod utils;

use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    control_groups::ControlGroups,
    spawn,
};

#[test]
fn assign_then_recall_restores_selection() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, b) = spawn::spawn_entity(world, "soldier", utils::pos(6, 5), Some(0)).unwrap();

    utils::select(&mut app, a);
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: b,
            mode: SelectMode::Add,
        },
    );
    utils::push_command(&mut app, PlayerCommand::AssignGroup { group: 0 });
    // Change the selection, then recall the group over it.
    utils::select(&mut app, a);
    utils::push_command(
        &mut app,
        PlayerCommand::RecallGroup {
            group: 0,
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![a, b]);
}

#[test]
fn append_group_adds_without_dropping_members() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, b) = spawn::spawn_entity(world, "soldier", utils::pos(6, 5), Some(0)).unwrap();

    utils::select(&mut app, a);
    utils::push_command(&mut app, PlayerCommand::AssignGroup { group: 1 });
    utils::select(&mut app, b);
    utils::push_command(&mut app, PlayerCommand::AppendGroup { group: 1 });
    utils::push_command(
        &mut app,
        PlayerCommand::RecallGroup {
            group: 1,
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![a, b]);
}

#[test]
fn recalled_group_excludes_destroyed_member() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    // A survivor and a critter that an adjacent enemy soldier kills.
    let (_, survivor) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, victim) = spawn::spawn_entity(world, "critter", utils::pos(10, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(11, 10), Some(1)).unwrap();

    utils::select(&mut app, survivor);
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: victim,
            mode: SelectMode::Add,
        },
    );
    utils::push_command(&mut app, PlayerCommand::AssignGroup { group: 2 });
    utils::run_ticks(&mut app, utils::APPLY);

    // The enemy auto-engages and kills the critter; its despawn sweeps the group.
    utils::run_ticks(&mut app, 12);
    assert_eq!(
        app.world().resource::<ControlGroups>().get(0, 2),
        &[survivor]
    );

    utils::push_command(
        &mut app,
        PlayerCommand::RecallGroup {
            group: 2,
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::selection(&app), vec![survivor]);
}

#[test]
fn recalling_empty_group_keeps_selection() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();

    // Group 5 was never assigned; recalling it must not clear the current selection.
    utils::select(&mut app, a);
    utils::push_command(
        &mut app,
        PlayerCommand::RecallGroup {
            group: 5,
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![a]);
}

#[test]
fn out_of_range_group_command_is_ignored() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();

    // A group index past the valid range can arrive over the wire; the executor
    // must ignore it rather than panic on the bounds-checked accessor.
    utils::select(&mut app, a);
    utils::push_command(&mut app, PlayerCommand::AssignGroup { group: 200 });
    utils::push_command(
        &mut app,
        PlayerCommand::RecallGroup {
            group: 200,
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![a]);
}
