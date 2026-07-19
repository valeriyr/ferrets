//! Deterministic state checksum for desync detection.
//!
//! Hashes the simulation's authoritative state so peers can compare a single
//! `u64` per tick and detect divergence. Only deterministic, fixed-point/integer
//! state is folded in — never Bevy [`Entity`](bevy_ecs::entity::Entity) ids
//! (assigned non-deterministically) — and entities are visited in ascending
//! [`SimulationId`](crate::simulation_id::SimulationId) order so two peers hash in
//! the same sequence.
//!
//! The digest is xxHash64 (via `xxhash-rust`) with a fixed seed and explicit
//! little-endian field encoding. `std`'s `DefaultHasher` is deliberately avoided:
//! its algorithm is unspecified and may change between Rust releases, so peers on
//! different toolchains could disagree. xxHash64 is a fixed, specified algorithm,
//! so the same state always hashes to the same value regardless of compiler
//! version or platform endianness.

use bevy_ecs::world::World;
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use xxhash_rust::xxh64::Xxh64;

use crate::{
    components::{health::HealthComponent, location::LocationComponent},
    entity_index::EntityIndex,
    resources::PlayerResources,
};

/// How often, in ticks, a state checksum is sampled: often enough that a
/// divergence surfaces quickly, infrequent enough to stay cheap.
pub const CHECKSUM_INTERVAL: u32 = 8;

/// Fixed seed for the checksum hash. Any constant works as long as all peers
/// agree; `0` is the conventional choice.
const CHECKSUM_SEED: u64 = 0;

/// xxHash64 wrapper that feeds every field as little-endian bytes, so the digest
/// is stable across Rust versions and platform endianness.
struct Checksum(Xxh64);

impl Checksum {
    fn new() -> Self {
        Self(Xxh64::new(CHECKSUM_SEED))
    }

    fn write_u8(&mut self, value: u8) {
        self.0.update(&[value]);
    }

    fn write_u32(&mut self, value: u32) {
        self.0.update(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.0.update(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.0.update(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.0.update(&value.to_le_bytes());
    }

    fn write_fixed_uvec2(&mut self, value: FixedUVec2) {
        self.write_fixed_u64(value.x);
        self.write_fixed_u64(value.y);
    }

    fn write_fixed_vec2(&mut self, value: FixedVec2) {
        self.write_fixed_i64(value.x);
        self.write_fixed_i64(value.y);
    }

    fn write_fixed_u64(&mut self, value: FixedU64) {
        self.write_u64(value.to_bits());
    }

    fn write_fixed_i64(&mut self, value: FixedI64) {
        self.write_i64(value.to_bits());
    }

    /// Length-prefixed so `"go" + "ld"` can't collide with `"gold"`.
    fn write_str(&mut self, value: &str) {
        self.write_usize(value.len());
        self.0.update(value.as_bytes());
    }

    fn finish(&self) -> u64 {
        self.0.digest()
    }
}

/// Computes a checksum of the current simulation state. Two peers in sync produce
/// the same value; a mismatch means they have diverged.
pub fn state_checksum(world: &World) -> u64 {
    let mut hasher = Checksum::new();

    // Entities in id order: position, facing, and health are the state most
    // likely to diverge. Alive then dying, each sorted by SimulationId.
    let index = world.resource::<EntityIndex>();

    for (id, entity) in index.all_entries() {
        hasher.write_u32(id.0);

        let entity = world.entity(entity);

        if let Some(location) = entity.get::<LocationComponent>() {
            hasher.write_fixed_uvec2(location.position);
            hasher.write_fixed_vec2(location.facing);
        }
        if let Some(health) = entity.get::<HealthComponent>() {
            hasher.write_u32(health.current());
        }
    }

    for (player, kind, amount) in world.resource::<PlayerResources>().iter() {
        hasher.write_u8(player);
        hasher.write_str(kind);
        hasher.write_u32(amount);
    }

    hasher.finish()
}
