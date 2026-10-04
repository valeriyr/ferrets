//! `state_checksum` must be deterministic (same state → same digest), sensitive
//! to any state change, and stable across builds (locked to a known xxHash64).

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{pool::Pool, pool_def::PoolId};
use ferrets_math::{FixedU64, facing::Facing};
use ferrets_simulation::{
    checksum,
    components::{
        location::LocationComponent,
        owner::OwnerComponent,
        pools,
        turret::{TurretState, TurretsComponent},
    },
    entity_index::EntityIndex,
    resources::PlayerResources,
    session::player_id::PlayerId,
};

mod utils;

#[test]
fn empty_state_matches_known_xxh64_seed0() {
    // No entities and no resources means nothing is hashed, so the digest is
    // xxHash64's known empty-input value for seed 0. This locks the algorithm and
    // seed: a change here means peers on an incompatible checksum would falsely
    // desync, and is a deliberate protocol break.
    let mut world = World::new();
    world.insert_resource(EntityIndex::default());
    world.insert_resource(PlayerResources::new(0));

    assert_eq!(checksum::state_checksum(&world), 0xef46_db37_51d8_e999);
}

#[test]
fn identical_state_hashes_identically() {
    let mut first = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut first, 5, 5);
    let mut second = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut second, 5, 5);

    assert_eq!(
        checksum::state_checksum(&first),
        checksum::state_checksum(&second)
    );
}

#[test]
fn moving_entity_changes_checksum() {
    let mut here = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut here, 5, 5);
    let mut there = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut there, 6, 5);

    assert_ne!(
        checksum::state_checksum(&here),
        checksum::state_checksum(&there)
    );
}

#[test]
fn turning_entity_changes_checksum() {
    // The look is part of the state the checksum samples, so a body that has come
    // round is a different state — which is what catches a peer whose unit turned
    // the other way.
    let mut facing = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut facing, 5, 5);
    let mut turned = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut turned, 5, 5);
    face(&mut turned, Facing::NORTH);

    assert_ne!(
        checksum::state_checksum(&facing),
        checksum::state_checksum(&turned)
    );
}

#[test]
fn aiming_gun_changes_checksum() {
    // The bearing is state of its own: a body standing exactly where its peer's
    // stands, with a gun round the other way, is about to shoot something else.
    let mut aimed = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut aimed, 5, 5);
    mount_gun(&mut aimed, Facing::NORTH);
    let mut turned = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut turned, 5, 5);
    mount_gun(&mut turned, Facing::EAST);

    assert_ne!(
        checksum::state_checksum(&aimed),
        checksum::state_checksum(&turned)
    );
}

#[test]
fn changing_health_changes_checksum() {
    let mut whole = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut whole, 5, 5);
    let mut hurt = utils::world([Pool::health(30)]);
    let unit = utils::spawn_unit(&mut hurt, 5, 5);
    pools::drain(&mut hurt, unit, PoolId::HEALTH, FixedU64::from_num(10));

    assert_ne!(
        checksum::state_checksum(&whole),
        checksum::state_checksum(&hurt)
    );
}

#[test]
fn changing_owner_changes_checksum() {
    // Whose an entity is decides what it may be told to do next, and a capture
    // moves it while the entity stands still — so it must show here rather
    // than through whatever it changes later.
    // Both worlds own the entity, so only the player it is owned BY can tell
    // them apart: an entity that merely gained an owner would move the digest
    // by the presence of the component alone.
    let mut mine = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut mine, 5, 5);
    own(&mut mine, 0);
    let mut theirs = utils::world([Pool::health(30)]);
    utils::spawn_unit(&mut theirs, 5, 5);
    own(&mut theirs, 1);

    assert_ne!(
        checksum::state_checksum(&mine),
        checksum::state_checksum(&theirs)
    );
}

#[test]
fn changing_resources_changes_checksum() {
    let mut poor = utils::world([Pool::health(30)]);
    poor.resource_mut::<PlayerResources>().add(0, "gold", 100);
    let mut rich = utils::world([Pool::health(30)]);
    rich.resource_mut::<PlayerResources>().add(0, "gold", 150);

    assert_ne!(
        checksum::state_checksum(&poor),
        checksum::state_checksum(&rich)
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Hands the world's one entity to `player`.
fn own(world: &mut World, player: PlayerId) {
    let entity = only_entity(world);
    world.entity_mut(entity).insert(OwnerComponent::new(player));
}

/// Fits the world's one entity with a gun trained on `bearing`.
fn mount_gun(world: &mut World, bearing: Facing) {
    let entity = only_entity(world);
    world
        .entity_mut(entity)
        .insert(TurretsComponent(vec![TurretState::mounted(bearing)]));
}

/// Points the world's one entity a different way.
fn face(world: &mut World, facing: Facing) {
    let entity = only_entity(world);
    world
        .get_mut::<LocationComponent>(entity)
        .expect("a spawned unit stands somewhere")
        .facing = facing;
}

/// The world's one alive entity.
fn only_entity(world: &World) -> Entity {
    let alive = world.resource::<EntityIndex>().alive_entries();
    assert_eq!(alive.len(), 1, "the world holds one alive entity");
    alive[0].1
}
