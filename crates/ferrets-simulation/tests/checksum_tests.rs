//! `state_checksum` must be deterministic (same state → same digest), sensitive
//! to any state change, and stable across builds (locked to a known xxHash64).

use bevy_ecs::world::World;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_simulation::{
    checksum::state_checksum,
    components::{health::HealthComponent, location::LocationComponent},
    entity_index::EntityIndex,
    resources::PlayerResources,
    simulation_id::SimulationId,
};

#[test]
fn empty_state_matches_known_xxh64_seed0() {
    // No entities and no resources means nothing is hashed, so the digest is
    // xxHash64's known empty-input value for seed 0. This locks the algorithm and
    // seed: a change here means peers on an incompatible checksum would falsely
    // desync, and is a deliberate protocol break.
    let mut world = World::new();
    world.insert_resource(EntityIndex::default());
    world.insert_resource(PlayerResources::new(0));

    assert_eq!(state_checksum(&world), 0xef46_db37_51d8_e999);
}

#[test]
fn identical_state_hashes_identically() {
    assert_eq!(
        state_checksum(&world(100, 30, 5)),
        state_checksum(&world(100, 30, 5)),
    );
}

#[test]
fn moving_entity_changes_checksum() {
    assert_ne!(
        state_checksum(&world(100, 30, 5)),
        state_checksum(&world(100, 30, 6)),
    );
}

#[test]
fn changing_health_changes_checksum() {
    assert_ne!(
        state_checksum(&world(100, 30, 5)),
        state_checksum(&world(100, 20, 5)),
    );
}

#[test]
fn changing_resources_changes_checksum() {
    assert_ne!(
        state_checksum(&world(100, 30, 5)),
        state_checksum(&world(150, 30, 5)),
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A world with one alive entity (at `(x, 5)` with `hp` health) and `gold` for
/// player 0 — enough to exercise both the entity and resource hashing paths.
fn world(gold: u32, hp: u32, x: u32) -> World {
    let mut world = World::new();
    let entity = world
        .spawn((
            LocationComponent::new(uvec2(x, 5), FixedVec2::ZERO),
            HealthComponent::full(FixedU64::from_num(hp)),
        ))
        .id();
    let mut index = EntityIndex::default();
    index.insert_alive(SimulationId(1), entity);
    world.insert_resource(index);
    let mut resources = PlayerResources::new(1);
    resources.add(0, "gold", gold);
    world.insert_resource(resources);
    world
}

fn uvec2(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}
