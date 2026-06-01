use ferrets_math::FixedI64;
use ferrets_math::fixed_vec2::FixedVec2;

pub fn scalar(n: i32) -> FixedI64 {
    FixedI64::from_num(n)
}

pub fn vec2(x: i32, y: i32) -> FixedVec2 {
    FixedVec2::new(scalar(x), scalar(y))
}
