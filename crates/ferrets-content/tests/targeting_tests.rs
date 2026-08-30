//! The targeting-layer predicate: which weapons reach which victims. A weapon
//! always declares its layers — a weapon cannot be stated without them — so the
//! only default is on the victim side.

mod utils;

use ferrets_content::{attack::AttackDef, entity_type_def::EntityTypeDef, targeting};
use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};
use utils::{AIR, GROUND, WATER};

//
// ─── Defaults ─────────────────────────────────────────────────────────────────
//

#[test]
fn type_without_weapon_reaches_nothing() {
    // The fail-closed residue: `targets` is required on every armed type, so
    // `None` only ever means "no weapon", and no weapon reaches nothing.
    let bystander = walker("peasant", GROUND);
    for victim in [walker("grunt", GROUND), walker("ship", WATER), flier()] {
        assert!(
            !targeting::reaches(targets_of(&bystander), &victim),
            "a type with no weapon reached '{}'",
            victim.name
        );
    }
}

#[test]
fn undeclared_victim_is_answerable_where_it_lives() {
    assert_eq!(targeting::targetable(&flier()), LayerMask::from(AIR));
    assert_eq!(
        targeting::targetable(&walker("grunt", GROUND)),
        LayerMask::from(GROUND)
    );
}

//
// ─── Narrowed weapons ─────────────────────────────────────────────────────────
//

#[test]
fn narrowed_weapon_reaches_only_named_layers() {
    let melee = attacker("grunt", GROUND, utils::weapon(GROUND | WATER));

    assert!(targeting::reaches(
        targets_of(&melee),
        &walker("footman", GROUND)
    ));
    assert!(targeting::reaches(
        targets_of(&melee),
        &walker("ship", WATER)
    ));
    assert!(
        !targeting::reaches(targets_of(&melee), &flier()),
        "a ground-and-water weapon must not reach the air"
    );
}

#[test]
fn weapon_excluding_layer_still_reaches_multi_layer_victim() {
    // A tall keep holding water and air at once is answerable by anything that
    // reaches either, which is why the match is an intersection and not a
    // containment: a ground-and-water weapon still gets to shoot it.
    let melee = attacker("grunt", GROUND, utils::weapon(GROUND | WATER));
    let keep = utils::standing("sea_fortress", WATER | AIR);

    assert!(targeting::reaches(targets_of(&melee), &keep));
}

//
// ─── Declared targetability ───────────────────────────────────────────────────
//

#[test]
fn declared_targetability_overrides_occupation() {
    // The tower stands on the ground and blocks only the ground, but answers to
    // anti-air as well — the case occupation alone cannot express.
    let tower = utils::standing("watch_tower", GROUND).with_targetable(GROUND | AIR);
    let anti_air_only = attacker("interceptor", AIR, utils::weapon(AIR));
    let melee = attacker("grunt", GROUND, utils::weapon(GROUND));

    assert_eq!(targeting::targetable(&tower), GROUND | AIR);
    assert!(
        targeting::reaches(targets_of(&anti_air_only), &tower),
        "an air-only weapon must reach a tower that declares itself an air target"
    );
    assert!(
        targeting::reaches(targets_of(&melee), &tower),
        "declaring an extra layer must not cost the tower its ground answerability"
    );
}

#[test]
fn declared_targetability_can_exclude_own_ground() {
    // The declaration is an override, not a union: a thing standing on the
    // ground that names only the air is out of every ground weapon's reach,
    // and only anti-air answers it.
    let totem = utils::standing("spirit_totem", GROUND).with_targetable(AIR);
    let melee = attacker("grunt", GROUND, utils::weapon(GROUND));
    let anti_air = attacker("interceptor", AIR, utils::weapon(AIR));

    assert_eq!(targeting::targetable(&totem), LayerMask::from(AIR));
    assert!(
        !targeting::reaches(targets_of(&melee), &totem),
        "a ground weapon reached a victim that declared itself away from the ground"
    );
    assert!(targeting::reaches(targets_of(&anti_air), &totem));
}

#[test]
fn air_only_weapon_cannot_reach_plain_ground() {
    let anti_air_only = attacker("interceptor", AIR, utils::weapon(AIR));

    assert!(!targeting::reaches(
        targets_of(&anti_air_only),
        &walker("grunt", GROUND)
    ));
}

//
// ─── Panics ───────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a weapon's targets must not be empty")]
fn empty_targets_panics() {
    attacker("grunt", GROUND, utils::weapon(LayerMask::EMPTY));
}

#[test]
#[should_panic(expected = "entity type 'watch_tower' is targetable on no layers")]
fn empty_targetable_panics() {
    utils::standing("watch_tower", GROUND).with_targetable(LayerMask::EMPTY);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A mover on `occupation`, carrying no weapon.
fn walker(name: &str, occupation: LayerId) -> EntityTypeDef {
    utils::standing(name, occupation).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    )
}

/// A mover on `occupation` carrying `weapon`.
fn attacker(name: &str, occupation: LayerId, weapon: AttackDef) -> EntityTypeDef {
    walker(name, occupation).with_attack(weapon, 5, 1, 5, 10, 5)
}

/// A flier: a mover living on the air layer alone.
fn flier() -> EntityTypeDef {
    walker("flier", AIR)
}

/// What the weapon `def` points reaches, or nothing at all where it points none.
fn targets_of(def: &EntityTypeDef) -> LayerMask {
    def.attack
        .as_ref()
        .map_or(LayerMask::EMPTY, |attack| attack.weapon().targets())
}
