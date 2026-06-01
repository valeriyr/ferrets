//! Fixed-point math primitives for deterministic RTS simulation.

pub mod fixed_rect;
pub mod fixed_urect;
pub mod fixed_uvec2;
pub mod fixed_vec2;

/// Signed 64-bit fixed-point scalar (32 integer bits, 32 fractional bits).
///
/// Suitable for velocities, directions, and offsets.
pub type FixedI64 = fixed::types::I32F32;

/// Unsigned 64-bit fixed-point scalar (32 integer bits, 32 fractional bits).
///
/// Suitable for positions, sizes, and distances.
pub type FixedU64 = fixed::types::U32F32;
