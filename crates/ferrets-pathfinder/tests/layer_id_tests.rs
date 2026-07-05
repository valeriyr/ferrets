use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_stores_bit_value() {
    assert_eq!(*LayerId::new(1), 1u32);
    assert_eq!(*LayerId::new(4), 4u32);
    assert_eq!(*LayerId::new(1 << 31), 1 << 31);
}

#[test]
#[should_panic(expected = "non-zero power of two")]
fn new_panics_on_zero() {
    LayerId::new(0);
}

#[test]
#[should_panic(expected = "non-zero power of two")]
fn new_panics_on_non_power_of_two() {
    LayerId::new(3);
}

#[test]
fn new_is_usable_in_const_context() {
    const GROUND: LayerId = LayerId::new(1);
    assert_eq!(*GROUND, 1u32);
}

#[test]
fn from_u32_is_equivalent_to_new() {
    assert_eq!(LayerId::from(8u32), LayerId::new(8));
}

#[test]
#[should_panic(expected = "non-zero power of two")]
fn from_u32_panics_on_zero() {
    let _ = LayerId::from(0u32);
}

#[test]
#[should_panic(expected = "non-zero power of two")]
fn from_u32_panics_on_non_power_of_two() {
    let _ = LayerId::from(3u32);
}

//
// ─── BitOr / BitAnd — produce LayerMask ──────────────────────────────────────
//

#[test]
fn bitor_two_layers_produces_union_mask() {
    let ground = LayerId::new(1);
    let air = LayerId::new(2);

    assert_eq!(ground | air, LayerMask::from(3u32));
}

#[test]
fn bitor_same_layer_produces_single_bit_mask() {
    let layer = LayerId::new(4);

    assert_eq!(layer | layer, LayerMask::from(4u32));
}

#[test]
fn bitand_same_layer_produces_single_bit_mask() {
    let layer = LayerId::new(4);

    assert_eq!(layer & layer, LayerMask::from(4u32));
}

#[test]
fn bitand_different_layers_produces_empty_mask() {
    let ground = LayerId::new(1);
    let air = LayerId::new(2);

    assert_eq!(ground & air, LayerMask::EMPTY);
}

//
// ─── Into<LayerMask> ─────────────────────────────────────────────────────────
//

#[test]
fn into_mask_preserves_bit() {
    let layer = LayerId::new(8);
    let mask = LayerMask::from(layer);

    assert_eq!(*mask, 8u32);
}

//
// ─── Display ──────────────────────────────────────────────────────────────────
//

#[test]
fn display_shows_decimal_value() {
    assert_eq!(format!("{}", LayerId::new(1)), "1");
    assert_eq!(format!("{}", LayerId::new(16)), "16");
}
