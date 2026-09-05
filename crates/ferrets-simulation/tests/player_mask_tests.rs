//! A set of players held one bit per player: membership, enumeration, and the
//! bitwise arithmetic the field grid folds masks with.

use ferrets_simulation::session::player_mask::PlayerMask;

//
// ─── Membership ───────────────────────────────────────────────────────────────
//

#[test]
fn empty_mask_holds_nobody() {
    assert!(PlayerMask::EMPTY.is_empty());
    assert!(!PlayerMask::EMPTY.contains(0));
    assert_eq!(PlayerMask::EMPTY.players().count(), 0);
}

#[test]
fn mask_of_player_holds_that_player_alone() {
    let mask = PlayerMask::of(3);

    assert!(!mask.is_empty());
    assert!(mask.contains(3));
    assert!(!mask.contains(2));
    assert!(!mask.contains(4));
    assert_eq!(mask.players().collect::<Vec<_>>(), vec![3]);
}

#[test]
fn mask_of_last_player_that_fits_is_accepted() {
    let mask = PlayerMask::of(31);

    assert!(mask.contains(31));
    assert_eq!(mask.players().collect::<Vec<_>>(), vec![31]);
}

#[test]
#[should_panic(expected = "does not fit a PlayerMask")]
fn mask_of_player_beyond_width_panics() {
    PlayerMask::of(32);
}

#[test]
fn players_enumerate_in_ascending_order() {
    let mask = PlayerMask::of(7) | PlayerMask::of(0) | PlayerMask::of(2);

    assert_eq!(mask.players().collect::<Vec<_>>(), vec![0, 2, 7]);
}

//
// ─── Arithmetic ───────────────────────────────────────────────────────────────
//

#[test]
fn union_holds_players_of_both_sides() {
    let mask = PlayerMask::of(1) | PlayerMask::of(2);

    assert!(mask.contains(1));
    assert!(mask.contains(2));
    assert!(!mask.contains(0));
}

#[test]
fn intersection_holds_only_shared_players() {
    let left = PlayerMask::of(1) | PlayerMask::of(2);
    let right = PlayerMask::of(2) | PlayerMask::of(3);

    assert_eq!(left & right, PlayerMask::of(2));
    assert!((PlayerMask::of(1) & PlayerMask::of(3)).is_empty());
}

#[test]
fn complement_holds_everyone_else() {
    let others = !PlayerMask::of(1);

    assert!(!others.contains(1));
    assert!(others.contains(0));
    assert!(others.contains(31));
    assert_eq!(others & PlayerMask::of(1), PlayerMask::EMPTY);
}

#[test]
fn union_assignment_adds_players_in_place() {
    let mut mask = PlayerMask::of(0);
    mask |= PlayerMask::of(5);

    assert_eq!(mask.players().collect::<Vec<_>>(), vec![0, 5]);
}

#[test]
fn intersection_assignment_keeps_shared_players_in_place() {
    let mut mask = PlayerMask::of(0) | PlayerMask::of(5) | PlayerMask::of(9);
    mask &= PlayerMask::of(5) | PlayerMask::of(9);

    assert_eq!(mask.players().collect::<Vec<_>>(), vec![5, 9]);
}

#[test]
fn removing_players_by_complement_leaves_rest() {
    let mask = PlayerMask::of(0) | PlayerMask::of(1) | PlayerMask::of(2);
    let remaining = mask & !PlayerMask::of(1);

    assert_eq!(remaining.players().collect::<Vec<_>>(), vec![0, 2]);
}
