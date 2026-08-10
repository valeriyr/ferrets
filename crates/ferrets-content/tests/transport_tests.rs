//! Transport capability: whom a [`TransporterDef`] admits aboard, and the
//! admission-list validation at construction.

use ferrets_content::transport::{BoardingPolicy, PassengerConduct, PassengerFate, TransporterDef};

//
// ─── Admission ────────────────────────────────────────────────────────────────
//

#[test]
fn admits_candidate_by_type_name() {
    let transporter = bunker(["footman"]);

    assert!(transporter.admits("footman", |_| false));
}

#[test]
fn admits_candidate_by_tag() {
    let transporter = bunker(["infantry"]);

    assert!(transporter.admits("footman", |tag| tag == "infantry"));
}

#[test]
fn refuses_candidate_matching_no_entry() {
    let transporter = bunker(["footman", "infantry"]);

    assert!(!transporter.admits("catapult", |tag| tag == "siege"));
}

#[test]
fn refuses_tag_absent_from_admission_list() {
    let transporter = bunker(["infantry"]);

    // The predicate answers for the candidate's own tags; a tag the admission
    // list never names must not admit, however many tags the candidate has.
    assert!(!transporter.admits("catapult", |_| false));
}

//
// ─── Admission list ───────────────────────────────────────────────────────────
//

#[test]
fn carries_returns_entries_in_ascending_order() {
    let transporter = bunker(["infantry", "footman"]);

    let carries: Vec<&str> = transporter.carries().collect();

    assert_eq!(carries, ["footman", "infantry"]);
}

#[test]
#[should_panic(expected = "carries must not be empty")]
fn empty_admission_list_panics() {
    bunker(Vec::<String>::new());
}

#[test]
#[should_panic(expected = "carried names must not be empty")]
fn empty_admission_entry_panics() {
    bunker([""]);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn bunker(carries: impl IntoIterator<Item = impl Into<String>>) -> TransporterDef {
    TransporterDef::new(
        carries,
        BoardingPolicy::Own,
        PassengerFate::Eject,
        PassengerConduct::Shelter,
    )
}
