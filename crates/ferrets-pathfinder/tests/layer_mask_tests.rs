use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};

//
// ─── Constants ────────────────────────────────────────────────────────────────
//

#[test]
fn empty_has_no_bits_set() {
    assert_eq!(*LayerMask::EMPTY, 0u32);
}

//
// ─── Conversions ──────────────────────────────────────────────────────────────
//

#[test]
fn from_u32_stores_raw_value() {
    assert_eq!(*LayerMask::from(0b1011u32), 0b1011u32);
}

#[test]
fn from_layer_id_sets_correct_bit() {
    assert_eq!(*LayerMask::from(LayerId::new(4)), 4u32);
}

//
// ─── BitOr ────────────────────────────────────────────────────────────────────
//

#[test]
fn bitor_combines_two_masks() {
    let a = LayerMask::from(0b0101u32);
    let b = LayerMask::from(0b1010u32);

    assert_eq!(*(a | b), 0b1111u32);
}

#[test]
fn bitor_with_layer_id_adds_bit() {
    assert_eq!(
        *(LayerMask::from(LayerId::new(0b0001u32)) | LayerId::new(0b0100u32)),
        0b0101u32
    );
}

#[test]
fn bitor_assign_accumulates_layers() {
    let mut mask = LayerMask::EMPTY;

    mask |= LayerId::new(0b0001u32);
    mask |= LayerId::new(0b0100u32);

    assert_eq!(*mask, 0b0101u32);
}

#[test]
fn bitor_assign_with_mask() {
    let mut mask = LayerMask::from(0b0001u32);

    mask |= LayerMask::from(0b0110u32);

    assert_eq!(*mask, 0b0111u32);
}

//
// ─── BitAnd ───────────────────────────────────────────────────────────────────
//

#[test]
fn bitand_gives_intersection() {
    let a = LayerMask::from(0b1100u32);
    let b = LayerMask::from(0b1010u32);

    assert_eq!(*(a & b), 0b1000u32);
}

#[test]
fn bitand_with_layer_id_tests_membership() {
    let mask = LayerMask::from(0b0101u32);
    let ground = LayerId::new(0b0001u32);
    let air = LayerId::new(0b0010u32);

    assert_ne!(mask & ground, LayerMask::EMPTY); // ground is in mask
    assert_eq!(mask & air, LayerMask::EMPTY); // air is not in mask
}

#[test]
fn bitand_assign_masks_down() {
    let mut mask = LayerMask::from(0b1110u32);

    mask &= LayerMask::from(0b1010u32);

    assert_eq!(*mask, 0b1010u32);
}

//
// ─── Not ──────────────────────────────────────────────────────────────────────
//

#[test]
fn not_inverts_all_bits() {
    assert_eq!(!LayerMask::EMPTY, LayerMask::from(u32::MAX));
    assert_eq!(!LayerMask::from(u32::MAX), LayerMask::EMPTY);
}

//
// ─── PartialEq<u32> ───────────────────────────────────────────────────────────
//

#[test]
fn partial_eq_with_u32() {
    assert!(LayerMask::EMPTY == 0u32);
    assert!(LayerMask::from(7u32) == 7u32);
    assert!(LayerMask::from(7u32) != 0u32);
}

//
// ─── Display ──────────────────────────────────────────────────────────────────
//

#[test]
fn display_shows_binary() {
    assert_eq!(format!("{}", LayerMask::EMPTY), "0b0");
    assert_eq!(format!("{}", LayerMask::from(0b0110u32)), "0b110");
    assert_eq!(format!("{}", LayerMask::from(0b1u32)), "0b1");
}
