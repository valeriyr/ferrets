#![allow(dead_code)]

use ferrets_content::{
    attack::{AttackDef, Delivery, Weapon},
    entity_type_def::EntityTypeDef,
    location::Solidity,
    registry::ContentRegistry,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};

/// The navigation layers these tests register, in the order a registry mints
/// them: the first layer declared is `1`, and each next one takes the next bit.
pub const GROUND: LayerId = LayerId::new(1);
pub const WATER: LayerId = LayerId::new(2);
pub const AIR: LayerId = LayerId::new(4);

/// A fresh registry that already knows the "ground" navigation layer.
pub fn ground_registry() -> ContentRegistry {
    let mut registry = ContentRegistry::default();
    registry.register_layer("ground");
    registry
}

/// A one-cell solid entity occupying `occupation`, with nothing else declared.
pub fn standing(name: &str, occupation: impl Into<LayerMask>) -> EntityTypeDef {
    sized(name, occupation, CellSize::ONE)
}

/// A solid entity occupying `occupation` whose footprint spans `size`.
pub fn sized(name: &str, occupation: impl Into<LayerMask>, size: CellSize) -> EntityTypeDef {
    EntityTypeDef::new(name).with_location(occupation, size, Solidity::Solid)
}

/// A weapon reaching `targets` that lands its hit where it stands, aimed from the
/// body — the plainest one there is, for tests about anything but the weapon.
pub fn weapon(targets: impl Into<LayerMask>) -> AttackDef {
    AttackDef::new(Weapon::new(targets, Delivery::Instant, None))
}
