//! Content validation at registration: [`ContentRegistry::register`] validates
//! each definition against the content already registered and panics on any
//! inconsistency, so a referenced type must be registered before the type that
//! references it.

use ferrets_pathfinder::{nav_grid::LayerId, nav_size::NavSize};
use ferrets_simulation::{
    components::{
        location::Solidity,
        resource::{DepletionPolicy, HarvestData, HarvestVisibility},
    },
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
};

//
// ─── Identity ─────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'worker' is already registered")]
fn register_rejects_a_duplicate_type() {
    let mut registry = ContentRegistry::default();
    registry.register(worker());
    registry.register(worker());
}

//
// ─── Location ─────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'worker' has no location")]
fn register_rejects_missing_location() {
    ContentRegistry::default().register(EntityTypeDef::new("worker"));
}

//
// ─── Resource kinds ───────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_definitions_without_resources() {
    let mut registry = ContentRegistry::default();
    registry.register(worker());
}

#[test]
fn register_accepts_registered_kinds() {
    gold_registry_with(
        worker()
            .with_cost([("gold", 10)])
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_resource_carrier([("gold", HarvestData::new(5, 2, HarvestVisibility::Hidden))])
            .with_resource_storage(["gold"]),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its cost")]
fn register_rejects_unknown_cost_kind() {
    gold_registry_with(worker().with_cost([("wood", 10)]));
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource source")]
fn register_rejects_unknown_source_kind() {
    gold_registry_with(worker().with_resource_source("wood", DepletionPolicy::Destroy));
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource carrier")]
fn register_rejects_unknown_carrier_kind() {
    gold_registry_with(
        worker()
            .with_resource_carrier([("wood", HarvestData::new(5, 2, HarvestVisibility::Visible))]),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource storage")]
fn register_rejects_unknown_storage_kind() {
    gold_registry_with(worker().with_resource_storage(["gold", "wood"]));
}

#[test]
#[should_panic(expected = "kind must not be empty")]
fn empty_resource_kind_panics() {
    ContentRegistry::default().register_resource("");
}

//
// ─── Production catalogues ────────────────────────────────────────────────────
//

// Production catalogues (trained/built types) are checked by `validate()`, not at
// registration, so they may reference each other in any order — including cycles.

#[test]
fn validate_accepts_registered_production_catalogues() {
    let mut registry = ContentRegistry::default();

    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_train_time(4),
    );
    registry.register(
        EntityTypeDef::new("depot")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_build_time(6),
    );
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_trainer(["soldier"]),
    );
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_builder(["depot"]),
    );

    registry.validate();
}

#[test]
fn validate_accepts_a_production_cycle() {
    // The town hall trains the worker and the worker builds the town hall — a
    // legitimate cycle that no registration order can express, but `validate`
    // accepts because it checks against the complete registry.
    let mut registry = ContentRegistry::default();
    registry.register(
        EntityTypeDef::new("town_hall")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_build_time(6)
            .with_trainer(["worker"]),
    );
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_train_time(4)
            .with_builder(["town_hall"]),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'barracks' trains 'ghost', which is not a registered trainable type"
)]
fn validate_rejects_unknown_trained_type() {
    let mut registry = ContentRegistry::default();
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_trainer(["ghost"]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "trains 'statue', which is not a registered trainable type")]
fn validate_rejects_untrainable_trained_type() {
    let mut registry = ContentRegistry::default();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        NavSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_trainer(["statue"]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'worker' builds 'nexus', which is not a registered constructible type"
)]
fn validate_rejects_unknown_built_type() {
    let mut registry = ContentRegistry::default();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_builder(["nexus"]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "builds 'statue', which is not a registered constructible type")]
fn validate_rejects_unconstructible_built_type() {
    let mut registry = ContentRegistry::default();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        NavSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_builder(["statue"]),
    );
    registry.validate();
}

//
// ─── Corpse chains ────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_terminating_corpse_chains() {
    let mut registry = ContentRegistry::default();

    registry.register(
        EntityTypeDef::new("bones")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(2, None),
    );
    registry.register(
        EntityTypeDef::new("corpse")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(2, Some("bones")),
    );
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(3, Some("corpse")),
    );
}

#[test]
#[should_panic(expected = "entity type 'soldier' leaves an unregistered corpse type 'ghost'")]
fn register_rejects_unknown_corpse_type() {
    let mut registry = ContentRegistry::default();
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(3, Some("ghost")),
    );
}

#[test]
#[should_panic(expected = "leaves a corpse type 'statue' that has no dying phase")]
fn register_rejects_corpse_without_dying_phase() {
    let mut registry = ContentRegistry::default();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        NavSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(3, Some("statue")),
    );
}

#[test]
#[should_panic(
    expected = "uses 'bones' as a corpse type, but 'bones' defines live-gameplay data that remains never use"
)]
fn register_rejects_corpse_with_live_gameplay_data() {
    let mut registry = ContentRegistry::default();
    registry.register(
        EntityTypeDef::new("bones")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_health(10)
            .with_attack(1, 1, 1, 1)
            .with_dying(2, None),
    );
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(3, Some("bones")),
    );
}

#[test]
#[should_panic(expected = "leaves an unregistered corpse type 'bones'")]
fn register_cannot_form_a_corpse_cycle() {
    let mut registry = ContentRegistry::default();

    // A corpse cycle is unconstructible: a corpse type must be registered before
    // the type that leaves it, so the first member of any cycle fails because
    // its own corpse is not registered yet.
    registry.register(
        EntityTypeDef::new("corpse")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(2, Some("bones")),
    );
}

//
// ─── Race ─────────────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_a_registered_race() {
    let mut registry = ContentRegistry::default();
    registry.register_race("human");
    registry.register(worker().with_race("human"));
}

#[test]
#[should_panic(expected = "belongs to unregistered race 'orc'")]
fn register_rejects_an_unregistered_race() {
    let mut registry = ContentRegistry::default();
    registry.register(worker().with_race("orc"));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

const GROUND: LayerId = LayerId::new(1);

/// Registers `def` into a registry that already knows the "gold" resource kind.
fn gold_registry_with(def: EntityTypeDef) {
    let mut registry = ContentRegistry::default();
    registry.register_resource("gold");
    registry.register(def);
}

fn worker() -> EntityTypeDef {
    EntityTypeDef::new("worker").with_location(GROUND, NavSize::ONE, Solidity::Solid)
}
