//! `state_checksum` must be deterministic (same state → same digest), sensitive
//! to any state change, and stable across builds (locked to a known xxHash64).

use bevy_ecs::world::World;
use ferrets_math::{FixedU64, facing::Facing, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    checksum::state_checksum,
    components::{
        health::HealthComponent,
        location::LocationComponent,
        turret::{TurretState, TurretsComponent},
    },
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
fn turning_entity_changes_checksum() {
    // The look is part of the state the checksum samples, so a body that has come
    // round is a different state — which is what catches a peer whose unit turned
    // the other way.
    let mut turned = world(100, 30, 5);
    face(&mut turned, Facing::NORTH);

    assert_ne!(state_checksum(&world(100, 30, 5)), state_checksum(&turned));
}

#[test]
fn aiming_gun_changes_checksum() {
    // The bearing is state of its own: a body standing exactly where its peer's
    // stands, with a gun round the other way, is about to shoot something else.
    let mut aimed = world(100, 30, 5);
    mount_gun(&mut aimed, Facing::NORTH);
    let mut turned = world(100, 30, 5);
    mount_gun(&mut turned, Facing::EAST);

    assert_ne!(state_checksum(&aimed), state_checksum(&turned));
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
            LocationComponent::new(uvec2(x, 5), Facing::SOUTH),
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

/// Fits the world's one entity with a gun trained on `bearing`.
fn mount_gun(world: &mut World, bearing: Facing) {
    let entity = world
        .resource::<EntityIndex>()
        .alive(SimulationId(1))
        .expect("the world's entity");
    world
        .entity_mut(entity)
        .insert(TurretsComponent(vec![TurretState::mounted(bearing)]));
}

/// Points the world's one entity a different way.
fn face(world: &mut World, facing: Facing) {
    let mut query = world.query::<&mut LocationComponent>();
    for mut location in query.iter_mut(world) {
        location.facing = facing;
    }
}

fn uvec2(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}
