//! Control groups: per-player saved selections, with bounds-checked access.

use ferrets_simulation::{
    control_groups::{CONTROL_GROUP_COUNT, ControlGroups},
    simulation_id::SimulationId,
};

//
// ─── Assign, append, recall ─────────────────────────────────────────────────
//

#[test]
fn assign_then_get_returns_saved_ids() {
    let mut groups = ControlGroups::new(1);
    groups.assign(0, 3, ids(&[7, 9]));
    assert_eq!(groups.get(0, 3), ids(&[7, 9]));
}

#[test]
fn assign_replaces_previous_members() {
    let mut groups = ControlGroups::new(1);
    groups.assign(0, 0, ids(&[1, 2]));
    groups.assign(0, 0, ids(&[3]));
    assert_eq!(groups.get(0, 0), ids(&[3]));
}

#[test]
fn append_adds_without_duplicating() {
    let mut groups = ControlGroups::new(1);
    groups.assign(0, 1, ids(&[1, 2]));
    groups.append(0, 1, &ids(&[2, 3]));
    assert_eq!(groups.get(0, 1), ids(&[1, 2, 3]));
}

#[test]
fn groups_are_independent_per_player() {
    let mut groups = ControlGroups::new(2);
    groups.assign(0, 0, ids(&[1]));
    groups.assign(1, 0, ids(&[2]));
    assert_eq!(groups.get(0, 0), ids(&[1]));
    assert_eq!(groups.get(1, 0), ids(&[2]));
}

#[test]
fn remove_sweeps_id_from_every_group_of_every_player() {
    let mut groups = ControlGroups::new(2);
    groups.assign(0, 0, ids(&[1, 2]));
    groups.assign(0, 1, ids(&[2, 3]));
    groups.assign(1, 4, ids(&[2]));

    groups.remove(SimulationId(2));

    assert_eq!(groups.get(0, 0), ids(&[1]));
    assert_eq!(groups.get(0, 1), ids(&[3]));
    assert_eq!(groups.get(1, 4), ids(&[]));
}

//
// ─── Bounds ─────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "control group 10 out of range (0..10)")]
fn get_out_of_range_panics() {
    let groups = ControlGroups::new(1);
    groups.get(0, CONTROL_GROUP_COUNT);
}

#[test]
#[should_panic(expected = "control group 10 out of range (0..10)")]
fn assign_out_of_range_panics() {
    let mut groups = ControlGroups::new(1);
    groups.assign(0, CONTROL_GROUP_COUNT, ids(&[1]));
}

#[test]
#[should_panic(expected = "control group 10 out of range (0..10)")]
fn append_out_of_range_panics() {
    let mut groups = ControlGroups::new(1);
    groups.append(0, CONTROL_GROUP_COUNT, &ids(&[1]));
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

fn ids(values: &[u32]) -> Vec<SimulationId> {
    values.iter().copied().map(SimulationId).collect()
}
