//! The index every simulation id is looked up through: what each stage admits,
//! and the contracts it refuses to break.

use bevy_ecs::world::World;
use ferrets_simulation::{entity_index::EntityIndex, simulation_id::SimulationId};

//
// ─── Stages ─────────────────────────────────────────────────────────────────
//

#[test]
fn alive_entity_is_found_as_alive_and_as_anything() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();

    index.insert_alive(SimulationId(1), entity);

    assert_eq!(
        index.alive(SimulationId(1)),
        Some(entity),
        "what was registered alive is found among the living"
    );
    assert_eq!(
        index.any(SimulationId(1)),
        Some(entity),
        "and among everything the index holds"
    );
    assert_eq!(
        index.remains(SimulationId(1)),
        None,
        "what stands is not a body"
    );
}

#[test]
fn body_begins_its_life_dying() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();

    index.insert_remains(SimulationId(1), entity);

    assert_eq!(
        index.alive(SimulationId(1)),
        None,
        "a body never stood as itself"
    );
    assert_eq!(
        index.remains(SimulationId(1)),
        Some(entity),
        "a body is found among the remains a cast may aim at"
    );
    assert_eq!(
        index.any(SimulationId(1)),
        Some(entity),
        "and among everything the index holds"
    );
    assert_eq!(index.remains_count(), 1, "and the cap counts it");
}

#[test]
fn dying_entity_leaves_alive_set_for_dying_one() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_alive(SimulationId(1), entity);

    index.mark_dying(SimulationId(1));

    assert_eq!(
        index.alive(SimulationId(1)),
        None,
        "what began to die stopped being alive"
    );
    assert_eq!(
        index.any(SimulationId(1)),
        Some(entity),
        "and is still something the index knows"
    );
    assert_eq!(
        index.dying_entries(),
        vec![(SimulationId(1), entity)],
        "found among the dying"
    );
}

#[test]
fn body_that_stops_being_one_goes_on_dying() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_remains(SimulationId(1), entity);

    index.remove_remains(SimulationId(1));

    assert_eq!(
        index.remains(SimulationId(1)),
        None,
        "nothing may aim at what the decay has ended"
    );
    assert_eq!(index.remains_count(), 0, "and no cap counts it");
    assert_eq!(
        index.any(SimulationId(1)),
        Some(entity),
        "while the death it is in the middle of goes on"
    );
}

#[test]
fn dying_entity_leaves_index_with_its_body() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_remains(SimulationId(1), entity);

    index.remove_dying(SimulationId(1));

    assert_eq!(
        index.any(SimulationId(1)),
        None,
        "what left the index is gone from every set"
    );
    assert_eq!(
        index.remains_count(),
        0,
        "a body put in both sets leaves both"
    );
}

#[test]
fn entries_come_out_in_id_order_alive_before_dying() {
    let mut world = World::new();
    let entities: Vec<_> = (0..4).map(|_| world.spawn_empty().id()).collect();
    let mut index = EntityIndex::default();
    // Registered out of order: the index is what sorts them.
    index.insert_alive(SimulationId(3), entities[2]);
    index.insert_alive(SimulationId(1), entities[0]);
    index.insert_remains(SimulationId(4), entities[3]);
    index.insert_alive(SimulationId(2), entities[1]);
    index.mark_dying(SimulationId(2));

    assert_eq!(
        index.all_entries(),
        vec![
            (SimulationId(1), entities[0]),
            (SimulationId(3), entities[2]),
            (SimulationId(2), entities[1]),
            (SimulationId(4), entities[3]),
        ],
        "alive in id order, then dying in id order"
    );
}

//
// ─── Contracts ──────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "simulation ids are minted once")]
fn id_registered_twice_panics() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_alive(SimulationId(1), entity);

    index.insert_alive(SimulationId(1), entity);
}

#[test]
#[should_panic(expected = "simulation ids are minted once")]
fn body_registered_over_dying_id_panics() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_alive(SimulationId(1), entity);
    index.mark_dying(SimulationId(1));

    index.insert_remains(SimulationId(1), entity);
}

#[test]
#[should_panic(expected = "only an alive entity begins to die")]
fn dying_what_never_stood_panics() {
    let mut index = EntityIndex::default();

    index.mark_dying(SimulationId(1));
}

#[test]
#[should_panic(expected = "an alive entity leaves the index by dying first")]
fn removing_alive_entity_panics() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_alive(SimulationId(1), entity);

    index.remove_dying(SimulationId(1));
}

#[test]
#[should_panic(expected = "only a dying entity leaves the index")]
fn removing_what_is_gone_panics() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_alive(SimulationId(1), entity);
    index.mark_dying(SimulationId(1));
    index.remove_dying(SimulationId(1));

    index.remove_dying(SimulationId(1));
}

#[test]
#[should_panic(expected = "a body stops being one while it is still dying")]
fn ending_decay_of_what_is_not_dying_panics() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    let mut index = EntityIndex::default();
    index.insert_alive(SimulationId(1), entity);

    index.remove_remains(SimulationId(1));
}
