//! Fixed-point math primitives for deterministic RTS simulation.

pub mod fixed_rect;
pub mod fixed_vec2;

/// Signed 64-bit fixed-point scalar (32 integer bits, 32 fractional bits).
///
/// All simulation values use this type to guarantee bit-identical results
/// across platforms.
pub type FixedI64 = fixed::types::I32F32;
