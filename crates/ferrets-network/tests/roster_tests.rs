//! The peer ↔ slot mapping, including slots with no network peer.

use ferrets_network::roster::Roster;

//
// ─── All-networked rosters ──────────────────────────────────────────────────
//

#[test]
fn new_maps_every_slot_to_peer() {
    let roster = Roster::new(vec![10, 20, 30]);

    assert_eq!(roster.len(), 3);
    assert_eq!(roster.peer_of(1), Some(20));
    assert_eq!(roster.player_of(30), Some(2));
    assert!(roster.is_networked(0));
}

//
// ─── Rosters with AI / closed slots ───────────────────────────────────────────
//

#[test]
fn from_slots_marks_peerless_slots_as_not_networked() {
    // Slot 1 is an AI or closed seat: it occupies an index but has no peer.
    let roster = Roster::from_slots(vec![Some(10), None, Some(30)]);

    assert_eq!(roster.len(), 3);
    assert!(roster.is_networked(0));
    assert!(!roster.is_networked(1));
    assert!(roster.is_networked(2));
}

#[test]
fn peerless_slot_has_no_peer() {
    let roster = Roster::from_slots(vec![Some(10), None, Some(30)]);

    assert_eq!(roster.peer_of(1), None);
    // The gap does not shift later slots' peers.
    assert_eq!(roster.peer_of(2), Some(30));
    assert_eq!(roster.player_of(30), Some(2));
}

#[test]
fn unknown_peer_and_out_of_range_slot_resolve_to_none() {
    let roster = Roster::from_slots(vec![Some(10), None]);

    assert_eq!(roster.player_of(99), None);
    assert_eq!(roster.peer_of(5), None);
    assert!(!roster.is_networked(5));
}
