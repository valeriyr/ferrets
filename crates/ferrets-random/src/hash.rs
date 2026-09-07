//! Integer mixing: a small hash that scatters its inputs.

/// Mixes two numbers into one that varies wildly with either. The same inputs
/// always mix to the same result.
pub fn mix(a: u32, b: u32) -> u32 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}
