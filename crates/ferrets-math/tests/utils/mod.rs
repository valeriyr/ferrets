#![allow(dead_code)]

use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};

pub fn scalar(n: i32) -> FixedI64 {
    FixedI64::from_num(n)
}

pub fn vec2(x: i32, y: i32) -> FixedVec2 {
    FixedVec2::new(scalar(x), scalar(y))
}

pub fn uscalar(n: u32) -> FixedU64 {
    FixedU64::from_num(n)
}

pub fn uvec2(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(uscalar(x), uscalar(y))
}

/// A fixed-point value parsed from decimal digits.
pub fn fixed(text: &str) -> FixedU64 {
    text.parse()
        .unwrap_or_else(|_| panic!("'{text}' is a value"))
}

/// A signed fixed-point value parsed from decimal digits.
pub fn signed_fixed(text: &str) -> FixedI64 {
    text.parse()
        .unwrap_or_else(|_| panic!("'{text}' is a signed value"))
}
