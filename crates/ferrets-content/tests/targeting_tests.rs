//! The targeting-layer predicate: which weapons reach which victims. A weapon
//! always declares its layers — a weapon cannot be stated without them — so the
//! only default is on the victim side.

mod utils;

use ferrets_content::{
    attack::AttackDef,
    domain::{DomainDef, DomainId},
    entity_type_def::EntityTypeDef,
    layer::LayerLevel,
    location::Solidity,
    registry::ContentRegistry,
    targeting,
    terrains::Terrains,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_nav_grid::layer_mask::LayerMask;

//
// ─── Defaults ─────────────────────────────────────────────────────────────────
//

#[test]
fn type_without_weapon_reaches_nothing() {
    let registry = layered_registry();
    // The fail-closed residue: `targets` is required on every armed type, so
    // `None` only ever means "no weapon", and no weapon reaches nothing.
    let bystander = walker("peasant", utils::GROUND_DOMAIN);
    for victim in [
        walker("grunt", utils::GROUND_DOMAIN),
        walker("ship", utils::WATER_DOMAIN),
        flier(),
    ] {
        assert!(
            !targeting::reaches(&registry, targets_of(&bystander), &victim),
            "a type with no weapon reached '{}'",
            victim.name
        );
    }
}

#[test]
fn undeclared_victim_is_answerable_where_it_stands() {
    let registry = layered_registry();
    assert_eq!(
        targeting::targetable(&registry, &flier()),
        LayerMask::from(utils::AIR)
    );
    assert_eq!(
        targeting::targetable(&registry, &walker("grunt", utils::GROUND_DOMAIN)),
        LayerMask::from(utils::GROUND)
    );
    assert_eq!(
        targeting::targetable(&registry, &keep()),
        LayerMask::from(utils::WATER),
        "a body occupying more than its domain's layer is answerable only where it stands"
    );
}

//
// ─── Narrowed weapons ─────────────────────────────────────────────────────────
//

#[test]
fn narrowed_weapon_reaches_only_named_layers() {
    let registry = layered_registry();
    let melee = attacker(
        "grunt",
        utils::GROUND_DOMAIN,
        utils::weapon(utils::GROUND | utils::WATER),
    );

    assert!(targeting::reaches(
        &registry,
        targets_of(&melee),
        &walker("footman", utils::GROUND_DOMAIN)
    ));
    assert!(targeting::reaches(
        &registry,
        targets_of(&melee),
        &walker("ship", utils::WATER_DOMAIN)
    ));
    assert!(
        !targeting::reaches(&registry, targets_of(&melee), &flier()),
        "a ground-and-water weapon must not reach the air"
    );
}

#[test]
fn weapon_excluding_layer_still_reaches_multi_layer_victim() {
    let registry = layered_registry();
    // A tall keep declared answerable on water and air at once is answerable by
    // anything that reaches either, which is why the match is an intersection
    // and not a containment: a ground-and-water weapon still gets to shoot it.
    let melee = attacker(
        "grunt",
        utils::GROUND_DOMAIN,
        utils::weapon(utils::GROUND | utils::WATER),
    );

    assert!(targeting::reaches(
        &registry,
        targets_of(&melee),
        &keep().with_targetable(utils::WATER | utils::AIR)
    ));
}

//
// ─── Declared targetability ───────────────────────────────────────────────────
//

#[test]
fn declared_targetability_overrides_standing_layer() {
    let registry = layered_registry();
    // The tower stands on the ground and blocks only the ground, but answers to
    // anti-air as well — the case what it occupies cannot express.
    let tower = utils::standing("watch_tower", utils::GROUND_DOMAIN)
        .with_targetable(utils::GROUND | utils::AIR);
    let anti_air_only = attacker("interceptor", utils::AIR_DOMAIN, utils::weapon(utils::AIR));
    let melee = attacker("grunt", utils::GROUND_DOMAIN, utils::weapon(utils::GROUND));

    assert_eq!(
        targeting::targetable(&registry, &tower),
        utils::GROUND | utils::AIR
    );
    assert!(
        targeting::reaches(&registry, targets_of(&anti_air_only), &tower),
        "an air-only weapon must reach a tower that declares itself an air target"
    );
    assert!(
        targeting::reaches(&registry, targets_of(&melee), &tower),
        "declaring an extra layer must not cost the tower its ground answerability"
    );
}

#[test]
fn declared_targetability_can_exclude_own_ground() {
    let registry = layered_registry();
    // The declaration is an override, not a union: a thing standing on the
    // ground that names only the air is out of every ground weapon's reach,
    // and only anti-air answers it.
    let totem = utils::standing("spirit_totem", utils::GROUND_DOMAIN).with_targetable(utils::AIR);
    let melee = attacker("grunt", utils::GROUND_DOMAIN, utils::weapon(utils::GROUND));
    let anti_air = attacker("interceptor", utils::AIR_DOMAIN, utils::weapon(utils::AIR));

    assert_eq!(
        targeting::targetable(&registry, &totem),
        LayerMask::from(utils::AIR)
    );
    assert!(
        !targeting::reaches(&registry, targets_of(&melee), &totem),
        "a ground weapon reached a victim that declared itself away from the ground"
    );
    assert!(targeting::reaches(&registry, targets_of(&anti_air), &totem));
}

#[test]
fn air_only_weapon_never_reaches_body_only_towering_into_air() {
    // The keep occupies the air over its water, blocking fliers there, but
    // stands on the water: occupying a layer is not being answerable on it.
    let registry = layered_registry();
    let anti_air_only = attacker("interceptor", utils::AIR_DOMAIN, utils::weapon(utils::AIR));

    assert!(!targeting::reaches(
        &registry,
        targets_of(&anti_air_only),
        &keep()
    ));
}

#[test]
fn air_only_weapon_cannot_reach_plain_ground() {
    let registry = layered_registry();
    let anti_air_only = attacker("interceptor", utils::AIR_DOMAIN, utils::weapon(utils::AIR));

    assert!(!targeting::reaches(
        &registry,
        targets_of(&anti_air_only),
        &walker("grunt", utils::GROUND_DOMAIN)
    ));
}

//
// ─── Panics ───────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a weapon's targets must not be empty")]
fn empty_targets_panics() {
    attacker(
        "grunt",
        utils::GROUND_DOMAIN,
        utils::weapon(LayerMask::EMPTY),
    );
}

#[test]
#[should_panic(expected = "entity type 'watch_tower' is targetable on no layers")]
fn empty_targetable_panics() {
    utils::standing("watch_tower", utils::GROUND_DOMAIN).with_targetable(LayerMask::EMPTY);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A mover in `domain`, carrying no weapon.
fn walker(name: &str, domain: DomainId) -> EntityTypeDef {
    utils::standing(name, domain).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    )
}

/// A mover in `domain` carrying `weapon`.
fn attacker(name: &str, domain: DomainId, weapon: AttackDef) -> EntityTypeDef {
    walker(name, domain).with_attack(weapon, 5, 1, 5, 10, 5)
}

/// A flier: a mover in the air domain alone.
fn flier() -> EntityTypeDef {
    walker("flier", utils::AIR_DOMAIN)
}

/// A keep in the water domain that occupies the air over it too.
fn keep() -> EntityTypeDef {
    EntityTypeDef::new("sea_fortress").with_location(
        utils::WATER_DOMAIN,
        CellSize::ONE,
        Solidity::Solid,
        utils::WATER | utils::AIR,
    )
}

/// A registry with the ground, water and air layers and a domain in each, as
/// the test helpers' layer and domain handles name them.
fn layered_registry() -> ContentRegistry {
    let mut registry = utils::ground_registry();
    let water = registry.register_layer("water", LayerLevel::At(2), &[]);
    let air = registry.register_layer("air", LayerLevel::At(3), &[]);
    assert_eq!((water, air), (utils::WATER, utils::AIR));
    assert_eq!(
        registry.register_domain("water", DomainDef::new(water, Terrains::Any, None)),
        utils::WATER_DOMAIN
    );
    assert_eq!(
        registry.register_domain("air", DomainDef::new(air, Terrains::Any, None)),
        utils::AIR_DOMAIN
    );
    registry
}

/// What the weapon `def` points reaches, or nothing at all where it points none.
fn targets_of(def: &EntityTypeDef) -> LayerMask {
    def.attack
        .as_ref()
        .map_or(LayerMask::EMPTY, |attack| attack.weapon().targets())
}
