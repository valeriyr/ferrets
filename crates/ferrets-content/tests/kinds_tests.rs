//! Which entities a rule names: the one filter every capability spells the same
//! way — what it admits, and what it refuses to be declared as.

use ferrets_content::{
    entity_type_def::EntityTypeDef,
    kinds::{Kind, Kinds},
    location::Solidity,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_pathfinder::layer_id::LayerId;

mod utils;

//
// ─── What a filter admits ─────────────────────────────────────────────────────
//

#[test]
fn names_candidate_by_its_type() {
    let kinds = Kinds::types(["footman"]);

    assert!(kinds.admits(&footman()));
    assert!(!kinds.admits(&catapult()));
}

#[test]
fn names_candidate_by_tag_it_wears() {
    let kinds = Kinds::tags(["infantry"]);

    assert!(kinds.admits(&footman()));
    assert!(!kinds.admits(&catapult()));
}

#[test]
fn names_candidate_matching_any_one_entry() {
    let kinds = Kinds::only([
        Kind::Type("catapult".to_string()),
        Kind::Tag("infantry".to_string()),
    ]);

    assert!(kinds.admits(&footman()), "by the tag");
    assert!(kinds.admits(&catapult()), "by the type name");
}

#[test]
fn type_and_tag_of_one_name_are_told_apart() {
    // The two vocabularies may share a name, which is why an entry says which
    // one it is drawn from rather than being matched against both.
    let by_type = Kinds::types(["infantry"]);
    let by_tag = Kinds::tags(["infantry"]);
    let named_infantry = standing("infantry", []);

    assert!(by_type.admits(&named_infantry));
    assert!(!by_type.admits(&footman()), "footman is not named infantry");
    assert!(by_tag.admits(&footman()));
    assert!(
        !by_tag.admits(&named_infantry),
        "a type named infantry wears no such tag"
    );
}

#[test]
fn any_names_everything() {
    assert!(Kinds::Any.admits(&footman()));
    assert!(Kinds::Any.admits(&catapult()));
}

//
// ─── What a filter refuses to be ──────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "kinds must name something")]
fn naming_nothing_panics() {
    Kinds::only([]);
}

#[test]
#[should_panic(expected = "a kind name must not be empty")]
fn empty_name_panics() {
    Kinds::types([""]);
}

#[test]
#[should_panic(expected = "kinds name entity type 'footman' twice")]
fn naming_one_type_twice_panics() {
    Kinds::types(["footman", "footman"]);
}

#[test]
#[should_panic(expected = "kinds name tag 'infantry' twice")]
fn naming_one_tag_twice_panics() {
    Kinds::tags(["infantry", "infantry"]);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

const GROUND: LayerId = LayerId::new(1);

/// A footman: an entity type of that name, wearing the `infantry` tag.
fn footman() -> EntityTypeDef {
    standing("footman", ["infantry"])
}

/// A catapult, wearing `siege` — named by nothing that names a footman.
fn catapult() -> EntityTypeDef {
    standing("catapult", ["siege"])
}

fn standing(name: &str, tags: impl IntoIterator<Item = &'static str>) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND, CellSize::ONE, Solidity::Solid)
        .with_tags(tags)
}
