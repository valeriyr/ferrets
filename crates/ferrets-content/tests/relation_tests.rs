//! Relations between owners and the sets of them a rule names.

use ferrets_content::relation::{Relation, Relations};

// ─── Relation ────────────────────────────────────────────────────────────────

#[test]
fn relation_predicates_name_their_arms() {
    // Own, ally, enemy and neutral, in that order.
    let arms = [
        Relation::Own,
        Relation::Ally,
        Relation::Enemy,
        Relation::Neutral,
    ];
    assert_eq!(arms.map(Relation::is_own), [true, false, false, false]);
    assert_eq!(arms.map(Relation::is_friendly), [true, true, false, false]);
    assert_eq!(arms.map(Relation::is_enemy), [false, false, true, false]);
}

// ─── Relations ───────────────────────────────────────────────────────────────

#[test]
fn set_contains_exactly_relations_joined_into_it() {
    let friendly = Relation::Own | Relation::Ally;
    // Own, ally, enemy and neutral, in that order.
    let arms = [
        Relation::Own,
        Relation::Ally,
        Relation::Enemy,
        Relation::Neutral,
    ];
    assert_eq!(
        arms.map(|arm| friendly.contains(arm)),
        [true, true, false, false]
    );
    assert_eq!(
        arms.map(|arm| Relations::from(Relation::Neutral).contains(arm)),
        [false, false, false, true]
    );
}

#[test]
fn named_sets_join_their_relations() {
    assert_eq!(Relations::FRIENDLY, Relation::Own | Relation::Ally);
    assert_eq!(
        Relations::EVERY,
        Relation::Own | Relation::Ally | Relation::Enemy | Relation::Neutral
    );
}

#[test]
fn empty_set_names_no_relation() {
    assert!(Relations::NONE.is_empty());
    assert_eq!(Relations::NONE.iter().count(), 0);
    assert!(!Relations::from(Relation::Enemy).is_empty());
    assert_eq!(Relations::NONE | Relation::Enemy, Relation::Enemy.into());
}

#[test]
fn joining_relation_already_named_changes_nothing() {
    let friendly = Relations::FRIENDLY;
    assert_eq!(friendly | Relation::Own, friendly);
    assert_eq!(friendly | Relations::FRIENDLY, friendly);
}

#[test]
fn sets_intersect_only_through_shared_relation() {
    let friendly = Relations::FRIENDLY;
    assert!(friendly.intersects(Relation::Ally | Relation::Enemy));
    assert!(!friendly.intersects(Relation::Enemy | Relation::Neutral));
    assert!(!friendly.intersects(Relations::NONE));
}

#[test]
fn relations_list_in_declared_order() {
    let every = Relation::Neutral | Relation::Enemy | Relation::Ally | Relation::Own;
    assert_eq!(
        every.iter().collect::<Vec<_>>(),
        [
            Relation::Own,
            Relation::Ally,
            Relation::Enemy,
            Relation::Neutral
        ]
    );
}
